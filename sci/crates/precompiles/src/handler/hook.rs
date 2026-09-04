//! Pre-execution hook logic — runs against any revm `Context` and is therefore
//! independent of Base's `OpHandler` (which lives in `base-common-evm`, a crate that
//! already depends on us). See `handler/mod.rs` for the architecture rationale.

use std::{collections::HashMap, fmt::Debug};

use alloy_evm::{Database as AlloyDatabase, EvmInternals};
use alloy_primitives::{Address, U256};
use revm::{
    context_interface::{
        Cfg, ContextTr, Database, JournalTr, Transaction,
        result::{FromStringError, InvalidTransaction},
    },
    handler::EvmTr,
    primitives::TxKind,
};
use tempo_contracts::precompiles::isTrippedCall;

use crate::{
    AccountKeychain, SciAgentState,
    handler::decode::classify_token_call,
    storage::{Handler, StorageCtx, evm::EvmPrecompileStorageProvider},
};

/// Outcome reported back to the handler wrapper.
///
/// `Pass` means the wrapper should fall through to normal EVM execution — either the tx
/// wasn't an agent tx, or all checks passed. `Reject` means the wrapper should fail the
/// tx with the contained error (a scope violation, quota exhaustion, or tripped agent).
#[derive(Debug)]
pub enum HookOutcome<E> {
    /// Hook completed; tx may proceed.
    Pass,
    /// Hook rejected the tx — surface this error via the `Handler::Error` path.
    Reject(E),
}

/// One decoded call from an AA transaction batch, as handed to [`run_aa_keychain_hook`].
///
/// The AA tx layout lives in `base-common-consensus` / `base-common-evm`, which depend on
/// this crate — so the `SciHandler` (which can read the tx's AA parts) decodes the batch
/// and passes it in, rather than this crate reaching back up to those types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AaCall {
    /// Call target (or CREATE).
    pub to: TxKind,
    /// Native value forwarded to the call (from `root`).
    pub value: U256,
    /// Calldata forwarded to the call.
    pub input: Vec<u8>,
}

/// Sets the keychain's transient `tx_origin` slot to the tx signer (and resets the
/// `transaction_key` slot) for a non-agent tx.
///
/// Tempo's handler does this unconditionally for every non-deposit tx so that keychain
/// admin operations (`authorizeKey` / `revokeKey` / `tripKey` / ...) invoked by a plain
/// tx see a non-zero `tx.origin` and pass the T2+ `ensure_admin_caller` check. AA agent
/// txs get the equivalent setup inside [`run_aa_keychain_hook`]; this covers every other
/// (non-agent) tx so a user calling the keychain directly still works.
pub fn set_keychain_tx_origin<EVM, ERROR>(evm: &mut EVM) -> Result<(), ERROR>
where
    EVM: EvmTr<
        Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>,
    >,
    ERROR: From<<<EVM::Context as ContextTr>::Db as Database>::Error>
        + FromStringError
        + From<InvalidTransaction>,
{
    let caller = evm.ctx().tx().caller();
    enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        let mut kc = AccountKeychain::default();
        kc.set_tx_origin(caller)?;
        kc.set_transaction_key(alloy_primitives::Address::ZERO)?;
        Ok(())
    })
    .map_err(|e| ERROR::from_string(format!("keychain tx_origin setup failed: {e:?}")))?;
    Ok(())
}

/// Reads the keychain's transient `tx_origin` slot — the symmetric reader for
/// [`set_keychain_tx_origin`]. Used by tests (and any host that needs to confirm what the
/// handler seeded) to observe the value `ensure_account_caller` will compare against
/// `msg_sender`. Returns `Address::ZERO` when no origin has been seeded for this tx.
pub fn keychain_tx_origin<EVM, ERROR>(evm: &mut EVM) -> Result<Address, ERROR>
where
    EVM: EvmTr<
        Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>,
    >,
    ERROR: From<<<EVM::Context as ContextTr>::Db as Database>::Error>
        + FromStringError
        + From<InvalidTransaction>,
{
    enter_keychain_storage(evm.ctx(), || -> crate::error::Result<Address> {
        AccountKeychain::default().tx_origin_raw()
    })
    .map_err(|e| ERROR::from_string(format!("keychain tx_origin read failed: {e:?}")))
}

/// AA-native keychain pre-execution hook (Plan A 2c) — the authorization gate.
///
/// Driven by the AA tx itself: the `SciHandler` passes the `root` account the calls
/// execute on behalf of, the `session_key` (the AA tx signer), and the decoded `calls`.
/// It enforces, for an AA tx whose `root` is set:
///
/// 1. **Authorization** — `keys[root][session_key]` must be an active access key; otherwise
///    the tx is rejected (this is what makes the 2a/2b `root`-execution + sponsored gas
///    safe: an arbitrary signer cannot act as, or spend the gas of, an unconsenting root).
/// 2. **Circuit breaker** — the session key must not be tripped.
/// 3. **Call scope** — every call must satisfy the access key's scope rules.
///
/// Transient writes (`transaction_key` / `tx_origin`) are wrapped in a journal checkpoint
/// so a rejection leaks no state. Spending-limit metering (pre-flight + deferred deduction,
/// including native value and gas via the `address(0)` sentinel) is layered on separately
/// (2c-ii).
///
/// Note (review finding L-7, intentional): an `enforce_limits` key whose root sponsors gas
/// (`fee_payer == root`) or whose batch moves native value MUST have an `address(0)`
/// sentinel limit row — without one `effective_remaining_limit` is zero and the whole tx
/// is rejected. Treating a missing row as "unlimited" would let gas/value spend bypass the
/// quota system entirely, so configure a sentinel limit when authorizing such keys.
pub fn run_aa_keychain_hook<EVM, ERROR>(
    evm: &mut EVM,
    root: Address,
    session_key: Address,
    calls: &[AaCall],
    gas_reservation: U256,
) -> Result<HookOutcome<ERROR>, ERROR>
where
    EVM: EvmTr<
        Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>,
    >,
    ERROR: From<<<EVM::Context as ContextTr>::Db as Database>::Error>
        + FromStringError
        + From<InvalidTransaction>,
{
    // Set tx_origin so keychain admin ops invoked within the batch see a non-zero origin,
    // and reset the transaction_key transient slot.
    enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        let mut kc = AccountKeychain::default();
        kc.set_tx_origin(session_key)?;
        kc.set_transaction_key(alloy_primitives::Address::ZERO)?;
        Ok(())
    })
    .map_err(|e| ERROR::from_string(format!("keychain tx_origin setup failed: {e:?}")))?;

    // 1. Authorization: keys[root][session_key] must be active.
    let is_active = enter_keychain_storage(evm.ctx(), || {
        AccountKeychain::default().key_is_active(root, session_key)
    })
    .map_err(|e| ERROR::from_string(format!("keychain probe failed: {e:?}")))?;
    if !is_active {
        // InvalidTransaction (not Custom) so the builder skips this tx instead of treating
        // it as a fatal payload-build error (which would wedge block production while the
        // rejected AA tx sits in the local-only pool and is retried every flashblock).
        return Ok(HookOutcome::Reject(ERROR::from(InvalidTransaction::Str(
            format!(
                "AA tx unauthorized: session key {session_key:?} has no active keychain key for root {root:?}",
            )
            .into(),
        ))));
    }

    // 2/3. CircuitBreaker + per-call scope, wrapped in a checkpoint so any partial transient
    //      write is rolled back on rejection.
    let checkpoint = evm.ctx().journal_mut().checkpoint();
    let hook_result = enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        let cb = SciAgentState::default();
        if cb.is_tripped(isTrippedCall { sessionKey: session_key })? {
            // Business error (not Fatal): a tripped key is a per-tx rejection, and the
            // system-error branch below must not escalate it to a block-build failure.
            return Err(
                tempo_contracts::precompiles::SciAgentStateError::key_tripped(session_key).into()
            );
        }

        let mut kc = AccountKeychain::default();
        kc.set_transaction_key(session_key)?;
        kc.set_tx_origin(session_key)?;

        // Per-call scope + accumulate the spend per token for the pre-flight limit check
        // (2c-ii). Native value (D2-B) and gas (D-gas) both meter against the `address(0)`
        // sentinel; recognized ERC-20 transfers/approves meter against the token (D3-B).
        let mut totals_per_token: HashMap<Address, U256> = HashMap::new();
        for call in calls {
            kc.validate_call_scope_for_transaction(root, session_key, &call.to, &call.input)?;
            if !call.value.is_zero() {
                let e = totals_per_token.entry(Address::ZERO).or_insert(U256::ZERO);
                *e = e.saturating_add(call.value);
            }
            if let TxKind::Call(target) = call.to
                && let Some((token, amount)) = classify_token_call(root, target, &call.input)
            {
                let e = totals_per_token.entry(token).or_insert(U256::ZERO);
                *e = e.saturating_add(amount);
            }
        }
        if !gas_reservation.is_zero() {
            let e = totals_per_token.entry(Address::ZERO).or_insert(U256::ZERO);
            *e = e.saturating_add(gas_reservation);
        }

        // Pre-flight (read-only): each token's batch total must fit the remaining quota,
        // honoring `enforce_limits`. Real deductions are deferred to
        // [`apply_aa_post_execution_deductions`] so a hook-passing, body-reverting tx costs
        // no quota (strong-R1).
        let key = kc.keys[root][session_key].read()?;
        if key.enforce_limits {
            let now = StorageCtx.timestamp().saturating_to::<u64>();
            for (token, total) in &totals_per_token {
                let remaining = kc.effective_remaining_limit(root, session_key, *token, now)?;
                if *total > remaining {
                    return Err(
                        tempo_contracts::precompiles::AccountKeychainError::spending_limit_exceeded()
                            .into(),
                    );
                }
            }
        }
        Ok(())
    });

    match hook_result {
        Ok(()) => {
            evm.ctx().journal_mut().checkpoint_commit();
            Ok(HookOutcome::Pass)
        }
        // System faults (DB failure, OOG, panic) are NOT per-tx rejections: silently
        // skipping the tx would mask a node-level problem (and a deterministic fault
        // would diverge sequencer/verifier). Propagate as a hard error.
        Err(e) if e.is_system_error() => {
            evm.ctx().journal_mut().checkpoint_revert(checkpoint);
            Err(ERROR::from_string(format!("SCI AA keychain hook system error: {e:?}")))
        }
        Err(e) => {
            evm.ctx().journal_mut().checkpoint_revert(checkpoint);
            // InvalidTransaction (not Custom) — keeps the builder skipping this rejected AA
            // tx instead of aborting the whole flashblock (CB-tripped / over-limit / scope).
            Ok(HookOutcome::Reject(ERROR::from(InvalidTransaction::Str(
                format!("SCI AA keychain hook rejected tx: {e:?}").into(),
            ))))
        }
    }
}

/// Applies the deferred spending-limit deductions for an AA agent tx (Plan A 2c-ii).
///
/// Pairs with [`run_aa_keychain_hook`]'s read-only pre-flight: the `SciHandler` calls this
/// from `execution_result` only when the batch executed successfully, so a hook-passing,
/// body-reverting tx costs no quota (strong-R1). The caller passes the already-decoded
/// `calls` and `gas_deduction` (the gas spend metered against the agent, in the sentinel
/// token's units; zero when the signer — not `root` — paid gas). Native value and gas both
/// deduct from the `address(0)` sentinel; recognized ERC-20 calls deduct per-token.
pub fn apply_aa_post_execution_deductions<EVM, ERROR>(
    evm: &mut EVM,
    root: Address,
    session_key: Address,
    calls: &[AaCall],
    gas_deduction: U256,
) -> Result<(), ERROR>
where
    EVM: EvmTr<
        Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>,
    >,
    ERROR: From<<<EVM::Context as ContextTr>::Db as Database>::Error>
        + FromStringError
        + From<InvalidTransaction>,
{
    enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        let mut kc = AccountKeychain::default();

        // Native value (D2-B) + gas (D-gas) → address(0) sentinel.
        let mut sentinel_total = gas_deduction;
        for call in calls {
            sentinel_total = sentinel_total.saturating_add(call.value);
        }
        if !sentinel_total.is_zero() {
            kc.verify_and_update_spending(root, session_key, Address::ZERO, sentinel_total)?;
        }

        // Recognized ERC-20 transfers/approves (D3-B, incl. transferWithMemo and
        // transferFrom(from == root)) → per-token.
        for call in calls {
            if let TxKind::Call(target) = call.to
                && let Some((token, amount)) = classify_token_call(root, target, &call.input)
            {
                kc.verify_and_update_spending(root, session_key, token, amount)?;
            }
        }
        Ok(())
    })
    .map_err(|e| ERROR::from_string(format!("apply AA deductions failed: {e:?}")))?;

    Ok(())
}

/// Constructs an [`EvmPrecompileStorageProvider`] from a generic [`ContextTr`] and runs
/// the closure inside its [`StorageCtx`].
///
/// We can't reuse [`StorageCtx::enter_ctx`] directly because it requires
/// `Cfg = CfgEnv<TempoHardfork>` while Base contexts use `Cfg = CfgEnv<OpSpecId>`. The
/// SCI hardfork ladder is orthogonal to Base's spec ladder; the level comes from
/// [`crate::SCI_LAUNCH_HARDFORK`] — the same value `install()` wires into the
/// precompiles, so the hook reads keychain state under identical packing/gating rules.
fn enter_keychain_storage<CTX, R>(ctx: &mut CTX, f: impl FnOnce() -> R) -> R
where
    CTX: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>,
{
    let (block, tx, cfg, journal, _chain, _local) = ctx.all_mut();
    let gas_params = cfg.gas_params().clone();
    let internals = EvmInternals::new(journal, block, cfg, tx);
    // 7-arg signature since Tempo v1.7.1 added `amsterdam_eip8037_enabled` for
    // EIP-8037 state-gas tracking; SCI hardcodes `false` (see
    // `sci-revm-shim` crate docs).
    let mut provider = EvmPrecompileStorageProvider::new(
        internals,
        u64::MAX,
        0,
        crate::SCI_LAUNCH_HARDFORK,
        false,
        false,
        gas_params,
    );
    StorageCtx::enter(&mut provider, f)
}
