//! Pre-execution hook logic — runs against any revm `Context` and is therefore
//! independent of Base's `OpHandler` (which lives in `base-common-evm`, a crate that
//! already depends on us). See `handler/mod.rs` for the architecture rationale.

use crate::{
    AccountKeychain, SciAgentState,
    handler::decode::classify_token_call,
    storage::{Handler, StorageCtx, evm::EvmPrecompileStorageProvider},
};
use alloy_evm::{Database as AlloyDatabase, EvmInternals};
use alloy_primitives::{Address, U256};
use std::collections::HashMap;
use revm::{
    context_interface::{
        Cfg, ContextTr, Database, JournalTr, Transaction,
        result::{FromStringError, InvalidTransaction},
    },
    handler::EvmTr,
    primitives::TxKind,
};
use std::fmt::Debug;
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_contracts::precompiles::isTrippedCall;

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
    EVM: EvmTr<Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>>,
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
pub fn run_aa_keychain_hook<EVM, ERROR>(
    evm: &mut EVM,
    root: Address,
    session_key: Address,
    calls: &[AaCall],
    gas_reservation: U256,
) -> Result<HookOutcome<ERROR>, ERROR>
where
    EVM: EvmTr<Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>>,
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
            return Err(crate::error::TempoPrecompileError::Fatal(format!(
                "agent session key {session_key:?} is tripped",
            )));
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
            if let TxKind::Call(target) = call.to {
                if let Some((token, amount)) = classify_token_call(target, &call.input) {
                    let e = totals_per_token.entry(token).or_insert(U256::ZERO);
                    *e = e.saturating_add(amount);
                }
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
            let now = StorageCtx::default().timestamp().saturating_to::<u64>();
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
    EVM: EvmTr<Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>>,
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

        // Recognized ERC-20 transfers/approves (D3-B, incl. transferWithMemo) → per-token.
        for call in calls {
            if let TxKind::Call(target) = call.to {
                if let Some((token, amount)) = classify_token_call(target, &call.input) {
                    kc.verify_and_update_spending(root, session_key, token, amount)?;
                }
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
/// SCI hardfork ladder is orthogonal to Base's spec ladder; SCI launches at
/// `TempoHardfork::T3` so we hardcode that.
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
        TempoHardfork::T3,
        false,
        false,
        gas_params,
    );
    StorageCtx::enter(&mut provider, f)
}
