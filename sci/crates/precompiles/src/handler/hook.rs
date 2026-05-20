//! Pre-execution hook logic — runs against any revm `Context` and is therefore
//! independent of Base's `OpHandler` (which lives in `base-common-evm`, a crate that
//! already depends on us). See `handler/mod.rs` for the architecture rationale.

use crate::{
    AccountKeychain, SciAgentState,
    handler::decode::{InnerCall, classify_token_call, decode_execute_batch},
    storage::{Handler, StorageCtx, evm::EvmPrecompileStorageProvider},
};
use alloy_evm::{Database as AlloyDatabase, EvmInternals};
use alloy_primitives::{Address, U256};
use std::collections::HashMap;
use revm::{
    bytecode::Bytecode,
    context::journaled_state::account::JournaledAccountTr,
    context_interface::{Cfg, ContextTr, Database, JournalTr, Transaction, result::FromStringError},
    handler::EvmTr,
    primitives::TxKind,
};
use std::fmt::Debug;
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_contracts::{
    precompiles::isTrippedCall, predeploys::SCI_AGENT_DELEGATOR_ADDRESS,
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

/// Runs the SCI Chain pre-execution hook over the current tx in `evm`.
///
/// Errors of type `ERROR` are only returned for system-level failures (database errors,
/// state corruption). Tx-level rejections (scope violations, quota exhaustion, tripped
/// agent) come back as `Ok(HookOutcome::Reject(err))`, where `err` is constructed via
/// `ERROR::from_string` so the wrapper can route it through revm's standard tx-error
/// pipeline.
///
/// Skipping deposit txs is the wrapper's responsibility — this function does not look at
/// `tx_type()` (which is Op-stack specific) and runs unconditionally.
pub fn run_pre_execution_hook<EVM, ERROR>(evm: &mut EVM) -> Result<HookOutcome<ERROR>, ERROR>
where
    EVM: EvmTr<Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>>,
    ERROR: From<<<EVM::Context as ContextTr>::Db as Database>::Error> + FromStringError,
{
    // ----- 1. Snapshot tx fields before borrowing the journal -----
    let (target_kind, caller, input) = {
        let ctx = evm.ctx();
        let tx = ctx.tx();
        (tx.kind(), tx.caller(), tx.input().clone())
    };

    // ----- 1.5. Set keychain's tx_origin for all non-deposit txs.
    //            Tempo's handler does this unconditionally so admin keychain ops
    //            (authorizeKey, revokeKey, etc.) see a non-zero tx.origin and pass
    //            the T2+ `ensure_admin_caller` check. We mirror that here. -----
    enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        let mut kc = AccountKeychain::default();
        kc.set_tx_origin(caller)?;
        // Reset transaction_key to zero (transient slot may persist across txs in
        // tests; in production each tx starts with zero, but being explicit is cheap).
        kc.set_transaction_key(alloy_primitives::Address::ZERO)?;
        Ok(())
    })
    .map_err(|e| ERROR::from_string(format!("keychain tx_origin setup failed: {e:?}")))?;

    let target = match target_kind {
        TxKind::Call(t) => t,
        // CREATE can't be 7702-delegated and isn't an agent flow.
        TxKind::Create => return Ok(HookOutcome::Pass),
    };

    // ----- 2. Read code(target) and parse EIP-7702 delegation header -----
    let delegate = {
        let acct = evm.ctx().journal_mut().load_account_with_code_mut(target)?;
        match acct.data.account().info.code.clone() {
            Some(Bytecode::Eip7702(eip)) => Some(eip.delegated_address),
            _ => None,
        }
    };
    let Some(delegate) = delegate else {
        return Ok(HookOutcome::Pass);
    };
    if delegate != SCI_AGENT_DELEGATOR_ADDRESS {
        return Ok(HookOutcome::Pass);
    }

    let root = target;
    let session_key = caller;

    // ----- 3. Probe keychain: is keys[root][session_key] still active? -----
    let is_active = enter_keychain_storage(evm.ctx(), || {
        let kc = AccountKeychain::default();
        kc.key_is_active(root, session_key)
    })
    .map_err(|e| ERROR::from_string(format!("keychain probe failed: {e:?}")))?;

    if !is_active {
        // 7702-delegated but no registered key → not an SCI agent tx, pass through.
        return Ok(HookOutcome::Pass);
    }

    // ----- 4. Decode batch; on selector mismatch fall back to a single-call probe -----
    let calls = decode_execute_batch(&input).unwrap_or_else(|| {
        vec![InnerCall {
            target,
            value: U256::ZERO,
            data: input.to_vec(),
        }]
    });

    // ----- 5. CB + transient signals + scope/deduct, wrapped in a journal checkpoint
    //         so partial writes auto-rollback on hook rejection (Q4 R1). -----
    let checkpoint = evm.ctx().journal_mut().checkpoint();

    let hook_result = enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        // 5a. CircuitBreaker
        let cb = SciAgentState::default();
        if cb.is_tripped(isTrippedCall { sessionKey: session_key })? {
            return Err(crate::error::TempoPrecompileError::Fatal(format!(
                "agent session key {session_key:?} is tripped",
            )));
        }

        // 5b. Seed transient slots so SCIAgentDelegator.execute()'s require passes and
        //     downstream keychain methods see the right access key.
        let mut kc = AccountKeychain::default();
        kc.set_transaction_key(session_key)?;
        kc.set_tx_origin(caller)?;

        // 5c. Per-call scope check + pre-flight spending check (read-only).
        //     Actual deduction is deferred to `apply_post_execution_deductions` so a
        //     tx that hook-passes but body-reverts doesn't lose quota (Q4 strong R1).
        let mut totals_per_token: HashMap<Address, U256> = HashMap::new();
        for call in &calls {
            kc.validate_call_scope_for_transaction(
                root,
                session_key,
                &TxKind::Call(call.target),
                &call.data,
            )?;
            if let Some((token, amount)) = classify_token_call(call.target, &call.data) {
                let entry = totals_per_token.entry(token).or_insert(U256::ZERO);
                *entry = entry.saturating_add(amount);
            }
        }
        // Pre-flight: verify each token's total would fit. We honor `enforce_limits`
        // by skipping the check when the key opts out — same as `verify_and_update_spending`.
        let now = StorageCtx::default().timestamp().saturating_to::<u64>();
        let key = kc.keys[root][session_key].read()?;
        if key.enforce_limits {
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
            // Hook succeeded — fold its writes into the parent (tx-level) checkpoint.
            // The tx body or revm's catch_error will roll the merged scope back if the
            // tx later fails for any reason; that delivers Q4 R1 auto-rollback for
            // post-hook revert paths.
            evm.ctx().journal_mut().checkpoint_commit();
            Ok(HookOutcome::Pass)
        }
        Err(e) => {
            // Hook rejected — discard everything we wrote since the checkpoint so a
            // partial multi-call batch doesn't leak quota deductions.
            evm.ctx().journal_mut().checkpoint_revert(checkpoint);
            Ok(HookOutcome::Reject(ERROR::from_string(format!(
                "SCI hook rejected tx: {e:?}",
            ))))
        }
    }
}

/// Applies deferred spending-limit deductions when the tx body executes successfully.
///
/// Pairs with [`run_pre_execution_hook`]: the hook does a read-only pre-flight check on
/// quota but doesn't write; if and only if the EVM body completes successfully, this
/// function writes the actual deductions, persisting the quota change to the journal
/// for commit. If the body reverts (or halts, or otherwise fails), this function is
/// **not** called by [`SciHandler::execution_result`], so the quota stays untouched —
/// delivering Q4 strong-R1 semantics.
///
/// The agent-tx signal carried across handler methods is the keychain's transient
/// `transaction_key` slot: the pre-execution hook sets it to the session key, this
/// function reads it. A zero value means "not an agent tx" and we exit immediately.
pub fn apply_post_execution_deductions<EVM, ERROR>(evm: &mut EVM) -> Result<(), ERROR>
where
    EVM: EvmTr<Context: ContextTr<Db: AlloyDatabase, Journal: JournalTr<Database: AlloyDatabase> + Debug>>,
    ERROR: From<<<EVM::Context as ContextTr>::Db as Database>::Error> + FromStringError,
{
    // Snapshot tx fields the same way pre-execution did.
    let (target_kind, _caller, input) = {
        let ctx = evm.ctx();
        let tx = ctx.tx();
        (tx.kind(), tx.caller(), tx.input().clone())
    };
    let target = match target_kind {
        TxKind::Call(t) => t,
        TxKind::Create => return Ok(()),
    };

    // Re-check 7702 delegation. Cheaper than persisting a flag across handler methods.
    let delegate = {
        let acct = evm.ctx().journal_mut().load_account_with_code_mut(target)?;
        match acct.data.account().info.code.clone() {
            Some(Bytecode::Eip7702(eip)) => Some(eip.delegated_address),
            _ => None,
        }
    };
    let Some(delegate) = delegate else {
        return Ok(());
    };
    if delegate != SCI_AGENT_DELEGATOR_ADDRESS {
        return Ok(());
    }

    // Read the keychain's transient `transaction_key` — set to the session key by the
    // pre-execution hook when an agent tx was detected. Zero means "no hook fired".
    let session_key = enter_keychain_storage(evm.ctx(), || {
        AccountKeychain::default().transaction_key_raw()
    })
    .map_err(|e| ERROR::from_string(format!("read transaction_key failed: {e:?}")))?;
    if session_key.is_zero() {
        return Ok(());
    }
    let root = target;

    // Re-decode batch; fall back to single-call probe for non-execute outer selectors.
    let calls = decode_execute_batch(&input).unwrap_or_else(|| {
        vec![InnerCall {
            target,
            value: U256::ZERO,
            data: input.to_vec(),
        }]
    });

    // Apply the actual deductions. Pre-flight already verified each fits.
    enter_keychain_storage(evm.ctx(), || -> crate::error::Result<()> {
        let mut kc = AccountKeychain::default();
        for call in &calls {
            if let Some((token, amount)) = classify_token_call(call.target, &call.data) {
                kc.verify_and_update_spending(root, session_key, token, amount)?;
            }
        }
        Ok(())
    })
    .map_err(|e| ERROR::from_string(format!("apply deductions failed: {e:?}")))?;

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
    let mut provider = EvmPrecompileStorageProvider::new(
        internals,
        u64::MAX,
        0,
        TempoHardfork::T3,
        false,
        gas_params,
    );
    StorageCtx::enter(&mut provider, f)
}
