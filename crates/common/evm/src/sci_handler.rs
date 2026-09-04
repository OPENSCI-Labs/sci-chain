//! Handler wrapper that inserts the SCI Chain pre-execution hook before tx execution.
//!
//! [`SciHandler`] wraps [`BaseHandler`] and delegates every method to it verbatim except
//! [`Handler::validate_against_state_and_deduct_caller`], which — after the inner
//! handler does the usual gas pre-pay / nonce bump — calls
//! [`sci_precompiles::run_aa_keychain_hook`] to enforce CircuitBreaker, call scope, and
//! spending-limit constraints for AA (`0x76`) agent txs whose `root` is set. Deposit
//! (system) txs short-circuit so OP-Stack predeploy ticks aren't subjected to keychain
//! checks.
//!
//! The hook logic itself lives in `sci-precompiles` so that crate stays independent of
//! `base-common-evm` — see `sci/crates/precompiles/src/handler/mod.rs` for the
//! architecture rationale.

use alloc::{boxed::Box, format, vec::Vec};

use revm::{
    bytecode::Bytecode,
    context_interface::{
        Block, Cfg, ContextTr, JournalTr, LocalContextTr, Transaction,
        result::{ExecutionResult, FromStringError, InvalidTransaction},
    },
    handler::{EthFrame, EvmTr, FrameResult, Handler, evm::FrameTr, handler::EvmTrError},
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::{
        CallInput, CallInputs, CallScheme, CallValue, CreateInputs, CreateScheme, Gas,
        InitialAndFloorGas, SharedMemory,
        interpreter::EthInterpreter,
        interpreter_action::{FrameInit, FrameInput},
    },
    primitives::{TxKind, U256},
    state::EvmState,
};
use sci_precompiles::{
    AaCall, HookOutcome, apply_aa_post_execution_deductions, run_aa_keychain_hook,
    set_keychain_tx_origin,
};

use crate::{
    BaseHaltReason, BaseSpecId, L1BlockInfo,
    handler::{BaseHandler, IsTxError},
    transaction::{AaTransactionParts, BaseTransactionError, BaseTxTr, DEPOSIT_TRANSACTION_TYPE},
};

/// SCI Chain handler wrapping Base's [`BaseHandler`]. See module docs.
#[derive(Debug, Clone, Default)]
pub struct SciHandler<EVM, ERROR, FRAME> {
    inner: BaseHandler<EVM, ERROR, FRAME>,
}

impl<EVM, ERROR, FRAME> SciHandler<EVM, ERROR, FRAME> {
    /// Creates a fresh [`SciHandler`] wrapping a fresh [`BaseHandler`].
    pub fn new() -> Self {
        Self { inner: BaseHandler::new() }
    }

    /// Normalizes a multi-call batch's [`FrameResult`] gas to the full tx gas limit,
    /// mirroring revm's [`Handler::last_frame_result`]: the whole `gas_limit` is marked
    /// spent, then `remaining` is credited back, and (on success) the accumulated refund
    /// is recorded. `remaining` is the gas left after the final call (or the failing
    /// call's leftover on revert; zero on halt).
    fn finalize_batch_gas(
        frame_result: &mut FrameResult,
        gas_limit: u64,
        remaining: u64,
        refund: i64,
    ) {
        let gas = frame_result.gas_mut();
        *gas = Gas::new_spent(gas_limit);
        gas.erase_cost(remaining);
        if refund > 0 {
            gas.record_refund(refund);
        }
    }
}

impl<EVM, ERROR, FRAME> SciHandler<EVM, ERROR, FRAME>
where
    EVM: EvmTr<
            Context: ContextTr<
                Db: alloy_evm::Database,
                Journal: JournalTr<State = EvmState, Database: alloy_evm::Database>
                             + core::fmt::Debug,
                Tx: BaseTxTr,
                Cfg: revm::context_interface::Cfg<Spec = BaseSpecId>,
                Chain = L1BlockInfo,
            >,
            Frame = FRAME,
        >,
    ERROR: EvmTrError<EVM> + From<BaseTransactionError> + FromStringError + IsTxError,
    FRAME: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
{
    /// Runs an AA tx's `calls[]` batch atomically, executing each [`Call`] as its own
    /// depth-0 frame via `run_frame`. The consensus path passes [`Handler::run_exec_loop`];
    /// the tracing path passes [`InspectorHandler::inspect_run_exec_loop`] so debug traces
    /// cover every call (not just the first). One outer journal checkpoint makes the batch
    /// atomic — any call's failure reverts the whole batch. Gas threads across calls; the
    /// returned [`FrameResult`] is the last call's, gas-normalized to the full tx limit.
    fn execute_aa_batch(
        &mut self,
        evm: &mut EVM,
        init_and_floor_gas: &InitialAndFloorGas,
        aa: AaTransactionParts,
        run_frame: fn(&mut Self, &mut EVM, FrameInit) -> Result<FrameResult, ERROR>,
    ) -> Result<FrameResult, ERROR> {
        let tx_gas_limit = evm.ctx().tx().gas_limit();
        // The tx is signed by the session key (= TxEnv.caller); the batch executes as
        // `root` when set, otherwise as the signer itself.
        let signer = evm.ctx().tx().caller();
        let caller = aa.root.unwrap_or(signer);
        // The deduct-caller step only loads the signer; when the batch executes as `root`
        // the journal must hold root's account too before any value transfer touches it
        // (revm's `transfer` assumes both ends are loaded — an unloaded `from` panics).
        evm.ctx().journal_mut().load_account(caller)?;

        // One outer checkpoint makes the whole batch atomic.
        let checkpoint = evm.ctx().journal_mut().checkpoint();
        let mut remaining_gas = tx_gas_limit.saturating_sub(init_and_floor_gas.initial_gas);
        let mut acc_refund: i64 = 0;
        let mut final_result: Option<FrameResult> = None;

        for call in &aa.calls {
            // Build this call's depth-0 frame (mirrors revm's `create_init_frame` but with
            // `caller` overridden to `root` and the per-call target/value/input).
            let frame_init = {
                let ctx = evm.ctx_mut();
                let mut memory =
                    SharedMemory::new_with_buffer(ctx.local().shared_memory_buffer().clone());
                memory.set_memory_limit(ctx.cfg().memory_limit());
                let frame_input = match call.to {
                    TxKind::Call(target) => {
                        let journal = ctx.journal_mut();
                        let account = &journal.load_account_with_code(target)?.info;
                        let known_bytecode = if let Some(Bytecode::Eip7702(eip)) = &account.code {
                            let delegated = eip.delegated_address;
                            let dacct = &journal.load_account_with_code(delegated)?.info;
                            Some((dacct.code_hash(), dacct.code.clone().unwrap_or_default()))
                        } else {
                            Some((account.code_hash(), account.code.clone().unwrap_or_default()))
                        };
                        FrameInput::Call(Box::new(CallInputs {
                            input: CallInput::Bytes(call.input.clone()),
                            return_memory_offset: 0..0,
                            gas_limit: remaining_gas,
                            bytecode_address: target,
                            known_bytecode,
                            target_address: target,
                            caller,
                            value: CallValue::Transfer(call.value),
                            scheme: CallScheme::Call,
                            is_static: false,
                        }))
                    }
                    TxKind::Create => FrameInput::Create(Box::new(CreateInputs::new(
                        caller,
                        CreateScheme::Create,
                        call.value,
                        call.input.clone(),
                        remaining_gas,
                    ))),
                };
                FrameInit { depth: 0, memory, frame_input }
            };

            let frame_result = run_frame(self, evm, frame_init)?;
            let result = frame_result.interpreter_result().result;
            if !result.is_ok() {
                // Fail-fast: revert the whole batch, surface the failing call's result.
                evm.ctx().journal_mut().checkpoint_revert(checkpoint);
                let mut fr = frame_result;
                // Revert refunds unused gas; halt consumes all gas.
                let remaining = if result.is_revert() { fr.gas().remaining() } else { 0 };
                Self::finalize_batch_gas(&mut fr, tx_gas_limit, remaining, 0);
                return Ok(fr);
            }
            remaining_gas = frame_result.gas().remaining();
            acc_refund = acc_refund.saturating_add(frame_result.gas().refunded());
            final_result = Some(frame_result);
        }

        evm.ctx().journal_mut().checkpoint_commit();
        let mut fr = final_result.expect("non-empty batch yields a result");
        Self::finalize_batch_gas(&mut fr, tx_gas_limit, remaining_gas, acc_refund);
        Ok(fr)
    }
}

impl<EVM, ERROR, FRAME> Handler for SciHandler<EVM, ERROR, FRAME>
where
    EVM: EvmTr<
            Context: ContextTr<
                Db: alloy_evm::Database,
                Journal: JournalTr<State = EvmState, Database: alloy_evm::Database>
                             + core::fmt::Debug,
                Tx: BaseTxTr,
                Cfg: revm::context_interface::Cfg<Spec = BaseSpecId>,
                Chain = L1BlockInfo,
            >,
            Frame = FRAME,
        >,
    ERROR: EvmTrError<EVM> + From<BaseTransactionError> + FromStringError + IsTxError,
    FRAME: FrameTr<FrameResult = FrameResult, FrameInit = FrameInit>,
{
    type Evm = EVM;
    type Error = ERROR;
    type HaltReason = BaseHaltReason;

    fn validate_env(&self, evm: &mut Self::Evm) -> Result<(), Self::Error> {
        self.inner.validate_env(evm)
    }

    fn validate_against_state_and_deduct_caller(
        &self,
        evm: &mut Self::Evm,
    ) -> Result<(), Self::Error> {
        // Plan A 2b — fee_payer sponsored gas. When an AA tx names a `fee_payer` (other
        // than the signing session key), gas is paid by the fee_payer, so the session key
        // may hold no funds. revm's inner deduct always charges `tx.caller` (the signer)
        // and bumps its nonce, so we pre-fund the signer from fee_payer with the MAX gas
        // the deduct can require (AA value is 0 — see `core.rs` — so this is pure gas),
        // then return the unspent remainder; the signer nets zero and fee_payer pays the
        // effective gas. The unused-gas refund is moved back to fee_payer in
        // [`reimburse_caller`].
        let signer = evm.ctx().tx().caller();
        // Sponsored gas is only allowed from the `root` account: the session key's right to
        // spend root's funds (gas included — D-gas) is authorized by the keychain
        // (`keys[root][session_key]`, enforced by the pre-execution hook in 2c). An
        // arbitrary third-party sponsor would need its own signature, which the AA tx does
        // not carry, so reject `fee_payer != root` to avoid draining an unconsenting account.
        let root = evm.ctx().tx().aa_parts().and_then(|a| a.root);
        let raw_fee_payer = evm.ctx().tx().aa_parts().and_then(|a| a.fee_payer);
        if raw_fee_payer.is_some() && raw_fee_payer != root {
            // InvalidTransaction (not from_string/Custom) so the builder skips this tx rather
            // than treating it as a fatal payload-build error (which wedges block production).
            return Err(ERROR::from(InvalidTransaction::Str(
                "AA fee_payer must equal root (sponsored gas is authorized via the keychain on \
                 the root account)"
                    .into(),
            )));
        }
        let fee_payer = raw_fee_payer.filter(|fp| *fp != signer);

        match fee_payer {
            Some(fp) => {
                // Sponsor everything Base's inner deduct charges the signer — L2 gas AND the
                // L1 data / operator cost. We can't know the exact total before the inner
                // runs (the L1 cost depends on the L1BlockInfo it fetches), so: pre-fund the
                // signer with the MAX L2 gas (gas_limit * max_fee; AA value is 0) from
                // fee_payer, run the inner deduct, then **bidirectionally** reconcile against
                // the signer's starting balance — return any excess, or cover any shortfall
                // (e.g. the L1 cost that exceeded the pre-funded gas) from fee_payer — so the
                // signer nets exactly zero and fee_payer bears the full effective cost.
                // Pre-fund the signer with the FULL amount Base's inner deduct requires it
                // to hold: the max L2 gas (`gas_limit * max_fee`; AA value is 0) PLUS the
                // L1 data / operator `additional_cost`. The inner deduct first subtracts
                // `additional_cost` (L1 + operator fee — see `handler.rs::tx_cost_with_tx`),
                // then `ensure_enough_balance` checks the *remaining* balance still covers
                // `max_balance_spending`. A funded signer's own balance absorbs the L1
                // portion and the reconcile below claws it back, but a truly 0-balance
                // signer underflows here before the reconcile runs — so it must be
                // pre-funded for BOTH. We mirror the inner's `L1BlockInfo` fetch (and cache
                // it on `chain`) so `tx_cost_with_tx` yields exactly the `additional_cost`
                // the inner will charge, keeping the reconcile delta to the unused-gas
                // remainder only.
                let prefund = {
                    let (block, tx, cfg, journal, chain, _local) = evm.ctx().all_mut();
                    let spec = cfg.spec();
                    if chain.l2_block != Some(block.number()) {
                        *chain = L1BlockInfo::try_fetch(journal.db_mut(), block.number(), spec)?;
                    }
                    let max_gas = tx
                        .max_balance_spending()
                        .map_err(|e| ERROR::from_string(format!("max gas overflow: {e:?}")))?;
                    let additional_cost = chain.tx_cost_with_tx(tx, spec).unwrap_or(U256::ZERO);
                    max_gas.saturating_add(additional_cost)
                };
                let signer_before = evm.ctx().journal_mut().load_account(signer)?.data.info.balance;
                if let Some(err) = evm.ctx().journal_mut().transfer(fp, signer, prefund)? {
                    return Err(ERROR::from_string(format!(
                        "fee_payer {fp:?} cannot cover gas: {err:?}"
                    )));
                }
                self.inner.validate_against_state_and_deduct_caller(evm)?;
                let signer_after = evm.ctx().journal_mut().load_account(signer)?.data.info.balance;
                let (from, to, amount) = if signer_after >= signer_before {
                    (signer, fp, signer_after - signer_before) // return excess to fee_payer
                } else {
                    (fp, signer, signer_before - signer_after) // fee_payer covers the shortfall
                };
                if amount != U256::ZERO {
                    if let Some(err) = evm.ctx().journal_mut().transfer(from, to, amount)? {
                        return Err(ERROR::from_string(format!(
                            "fee_payer gas reconcile failed: {err:?}"
                        )));
                    }
                }
            }
            None => self.inner.validate_against_state_and_deduct_caller(evm)?,
        }

        // Deposit (`0x7E`) txs bypass the SCI `0x76` agent keychain hook — they aren't agent
        // txs (no `aa_parts`, no `root`), so the scope/limit/CB gate below does not apply.
        // But we still seed the keychain `tx_origin` for them: this is the L1 censorship
        // escape hatch (Tier 2). A root owner whose txs the sequencer censors can force-include
        // a keychain admin call (`revokeKey` / `updateSpendingLimit` / ...) from L1 via
        // `OptimismPortal.depositTransaction`; that call executes on L2 with `msg.sender` = the
        // L1 EOA, and the keychain admin gate `ensure_account_caller` requires
        // `tx_origin == msg_sender` (non-zero) AND `transaction_key == 0`.
        // `set_keychain_tx_origin` seeds exactly that (origin = deposit `from`, key = 0).
        //
        // Safe for system deposits: `tx_origin`/`transaction_key` are transient (TSTORE) —
        // cleared at tx end, never committed to the state root — and system deposits (L1-info,
        // upgrades) never read them, so the seed is inert there. An EOA deposit gains only
        // self-admin over its own `keys[msg_sender]` (no agent-delegation powers, since
        // `transaction_key` stays 0). See `sci/docs/plan-a-l1-escape-hatch.md` §5 / §5.1.
        if evm.ctx().tx().tx_type() == DEPOSIT_TRANSACTION_TYPE {
            return set_keychain_tx_origin::<EVM, ERROR>(evm);
        }

        // Plan A 2c — AA agent-tx authorization. An AA tx with `root` set acts on behalf
        // of `root`, so the keychain must authorize the session key (`keys[root][signer]`)
        // and gate the batch (circuit breaker + per-call scope). Extract the batch here
        // (the hook crate can't read the AA tx env) and run the AA-native keychain hook.
        // AA txs without `root` (a plain batch executed as the signer) and every non-AA tx
        // are not agent traffic and fall through to normal execution with no keychain gate.
        let aa_calls: Option<Vec<AaCall>> = if root.is_some() {
            evm.ctx().tx().aa_parts().map(|parts| {
                parts
                    .calls
                    .iter()
                    .map(|c| AaCall { to: c.to, value: c.value, input: c.input.to_vec() })
                    .collect()
            })
        } else {
            None
        };

        match (root, aa_calls) {
            (Some(root_addr), Some(calls)) => {
                // D-gas: when fee_payer (== root) sponsors gas, reserve the max gas spend
                // (gas_limit * max_fee, pessimistic) against root's `address(0)` sentinel
                // limit in the pre-flight; when the signer pays gas it isn't root's spend.
                let gas_reservation = if raw_fee_payer == Some(root_addr) {
                    let (_block, tx, _cfg, _journal, _chain, _local) = evm.ctx().all_mut();
                    U256::from(tx.gas_limit()).saturating_mul(U256::from(tx.max_fee_per_gas()))
                } else {
                    U256::ZERO
                };
                match run_aa_keychain_hook::<EVM, ERROR>(
                    evm,
                    root_addr,
                    signer,
                    &calls,
                    gas_reservation,
                )? {
                    HookOutcome::Pass => Ok(()),
                    HookOutcome::Reject(err) => Err(err),
                }
            }
            // Non-agent traffic (non-AA tx, or AA tx without `root`): no keychain gate,
            // but still set the keychain's tx_origin so a plain tx calling the keychain
            // directly (authorizeKey/revokeKey/...) passes the T2+ ensure_admin_caller check.
            _ => set_keychain_tx_origin::<EVM, ERROR>(evm),
        }
    }

    fn last_frame_result(
        &mut self,
        evm: &mut Self::Evm,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.last_frame_result(evm, frame_result)
    }

    /// Consensus (non-inspecting) execution path.
    ///
    /// AA txs (type `0x76`) carry a `calls[]` batch and run it atomically via
    /// [`Self::execute_aa_batch`] using the standard [`Handler::run_exec_loop`]; every other
    /// tx type has no `aa` parts and delegates to Base's single-call execution. The tracing
    /// path mirrors this in [`InspectorHandler::inspect_execution`].
    fn execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &InitialAndFloorGas,
    ) -> Result<FrameResult, Self::Error> {
        match evm.ctx().tx().aa_parts().cloned().filter(|aa| !aa.calls.is_empty()) {
            Some(aa) => {
                self.execute_aa_batch(evm, init_and_floor_gas, aa, <Self as Handler>::run_exec_loop)
            }
            None => self.inner.execution(evm, init_and_floor_gas),
        }
    }

    fn reimburse_caller(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        // Plan A 2b — pair with the fee_payer pre-fund in
        // [`validate_against_state_and_deduct_caller`]: revm refunds unused gas to the
        // signer (`tx.caller`); move that refund on to fee_payer so it nets the effective
        // gas used and the signer nets zero.
        let signer = evm.ctx().tx().caller();
        let fee_payer =
            evm.ctx().tx().aa_parts().and_then(|a| a.fee_payer).filter(|fp| *fp != signer);

        let Some(fp) = fee_payer else {
            return self.inner.reimburse_caller(evm, frame_result);
        };

        let before = evm.ctx().journal_mut().load_account(signer)?.data.info.balance;
        self.inner.reimburse_caller(evm, frame_result)?;
        let after = evm.ctx().journal_mut().load_account(signer)?.data.info.balance;
        let refund = after.saturating_sub(before);
        if refund != U256::ZERO {
            if let Some(err) = evm.ctx().journal_mut().transfer(signer, fp, refund)? {
                return Err(ERROR::from_string(format!("fee_payer refund failed: {err:?}")));
            }
        }
        Ok(())
    }

    fn refund(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
        eip7702_refund: i64,
    ) {
        self.inner.refund(evm, frame_result, eip7702_refund)
    }

    fn reward_beneficiary(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.reward_beneficiary(evm, frame_result)
    }

    fn execution_result(
        &mut self,
        evm: &mut Self::Evm,
        frame_result: <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        // Q4 strong-R1 (spend) + always-on gas metering. The pre-execution hook does a
        // read-only pre-flight; this is where the real deductions land, split by outcome:
        //
        // - Token / native-value deductions apply only when the EVM body completed
        //   successfully — a hook-passing, body-reverting tx moved no funds, so it costs
        //   zero spend quota (strong-R1).
        // - The D-gas sentinel deduction applies REGARDLESS of body outcome: when
        //   `fee_payer == root` sponsors gas, a reverting batch still burns root's real
        //   ETH for gas, so the `address(0)` quota must track it. Otherwise a session key
        //   could drain root via deliberately-reverting batches without ever touching the
        //   limit (review finding M-1). The deduction (`gas_used * max_fee`) never exceeds
        //   the pre-flight reservation (`gas_limit * max_fee`), so it cannot fail a limit
        //   the hook already verified.
        //
        // Only AA agent txs (root set) carry deductions; deposit and non-agent txs have none.
        let root = evm.ctx().tx().aa_parts().and_then(|a| a.root);
        if let Some(root_addr) = root {
            let body_ok = frame_result.interpreter_result().result.is_ok();
            let signer = evm.ctx().tx().caller();
            let raw_fee_payer = evm.ctx().tx().aa_parts().and_then(|a| a.fee_payer);
            // D-gas: the gas the agent actually spent (gas_used * max_fee, pessimistic,
            // matching the pre-flight reservation), counted only when root sponsored gas.
            let gas_deduction = if raw_fee_payer == Some(root_addr) {
                let gas_used = frame_result.gas().used();
                let max_fee = evm.ctx().tx().max_fee_per_gas();
                U256::from(gas_used).saturating_mul(U256::from(max_fee))
            } else {
                U256::ZERO
            };
            // On a failed body the batch's transfers were rolled back: meter gas only.
            let calls: Vec<AaCall> = if body_ok {
                evm.ctx()
                    .tx()
                    .aa_parts()
                    .map(|parts| {
                        parts
                            .calls
                            .iter()
                            .map(|c| AaCall { to: c.to, value: c.value, input: c.input.to_vec() })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            if body_ok || !gas_deduction.is_zero() {
                apply_aa_post_execution_deductions::<EVM, ERROR>(
                    evm,
                    root_addr,
                    signer,
                    &calls,
                    gas_deduction,
                )?;
            }
        }
        self.inner.execution_result(evm, frame_result)
    }

    fn catch_error(
        &self,
        evm: &mut Self::Evm,
        err: Self::Error,
    ) -> Result<ExecutionResult<Self::HaltReason>, Self::Error> {
        self.inner.catch_error(evm, err)
    }
}

impl<EVM, ERROR> InspectorHandler for SciHandler<EVM, ERROR, EthFrame<EthInterpreter>>
where
    EVM: InspectorEvmTr<
            Context: ContextTr<
                Db: alloy_evm::Database,
                Journal: JournalTr<State = EvmState, Database: alloy_evm::Database>
                             + core::fmt::Debug,
                Tx: BaseTxTr,
                Cfg: revm::context_interface::Cfg<Spec = BaseSpecId>,
                Chain = L1BlockInfo,
            >,
            Frame = EthFrame<EthInterpreter>,
            Inspector: revm::inspector::Inspector<
                <<Self as Handler>::Evm as EvmTr>::Context,
                EthInterpreter,
            >,
        >,
    ERROR: EvmTrError<EVM> + From<BaseTransactionError> + FromStringError + IsTxError,
{
    type IT = EthInterpreter;

    /// Tracing execution path — mirror of [`Handler::execution`] so `debug_trace*` covers an
    /// AA tx's whole `calls[]` batch (not just the first call). AA txs run the batch via
    /// [`Self::execute_aa_batch`] with the inspecting [`InspectorHandler::inspect_run_exec_loop`];
    /// every other tx type takes the default single-frame inspector path.
    fn inspect_execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &InitialAndFloorGas,
    ) -> Result<FrameResult, Self::Error> {
        if let Some(aa) = evm.ctx().tx().aa_parts().cloned().filter(|aa| !aa.calls.is_empty()) {
            return self.execute_aa_batch(
                evm,
                init_and_floor_gas,
                aa,
                <Self as InspectorHandler>::inspect_run_exec_loop,
            );
        }

        // Default single-frame inspector path (non-AA / empty batch), matching the trait's
        // built-in `inspect_execution`.
        let gas_limit = evm.ctx().tx().gas_limit() - init_and_floor_gas.initial_gas;
        let first_frame_input = self.first_frame_input(evm, gas_limit)?;
        let mut frame_result = self.inspect_run_exec_loop(evm, first_frame_input)?;
        self.last_frame_result(evm, &mut frame_result)?;
        Ok(frame_result)
    }
}

#[cfg(test)]
mod tests {
    use base_common_chains::BaseUpgrade;
    use revm::{
        context::{CfgEnv, Context, TxEnv},
        context_interface::result::EVMError,
        database::InMemoryDB,
        handler::Handler,
        primitives::{Address, B256},
        state::AccountInfo,
    };
    use sci_precompiles::keychain_tx_origin;

    use super::*;
    use crate::{BaseSpecId, BaseTransaction, Builder, DefaultBase, L1BlockInfo};

    type TestError = EVMError<core::convert::Infallible, BaseTransactionError>;

    /// Builds an EVM holding a single deposit (`0x7E`) tx from `caller`, runs
    /// `SciHandler::validate_against_state_and_deduct_caller`, and returns the keychain
    /// `tx_origin` the handler seeded. `system` toggles the L1-info/upgrade flavor.
    fn deposit_seeded_tx_origin(caller: Address, system: bool) -> Address {
        let mut db = InMemoryDB::default();
        db.insert_account_info(
            caller,
            AccountInfo { balance: U256::from(1_000_000), ..Default::default() },
        );

        let mut builder = BaseTransaction::builder()
            .base(TxEnv::builder().caller(caller).gas_limit(100))
            .source_hash(B256::from([1u8; 32]));
        if system {
            builder = builder.is_system_transaction();
        }

        let ctx = Context::base()
            .with_db(db)
            .with_chain(L1BlockInfo::default())
            .with_tx(builder.build_fill())
            .with_cfg(CfgEnv::new_with_spec(BaseSpecId::new(BaseUpgrade::Regolith)));

        let mut evm = ctx.build_base();
        let handler = SciHandler::<_, TestError, EthFrame<EthInterpreter>>::new();
        handler.validate_against_state_and_deduct_caller(&mut evm).unwrap();
        keychain_tx_origin::<_, TestError>(&mut evm).unwrap()
    }

    /// Tier 2 (L1 escape hatch): a deposit tx must seed the keychain `tx_origin` to its
    /// caller, so a force-included keychain admin call (`revokeKey` / ...) from an L1 EOA
    /// passes `ensure_account_caller` (which requires `tx_origin == msg_sender`, non-zero).
    /// Before Tier 2 the deposit short-circuited and left `tx_origin == ZERO`.
    #[test]
    fn deposit_seeds_keychain_tx_origin() {
        let caller = Address::from([0x11; 20]);
        assert_eq!(
            deposit_seeded_tx_origin(caller, false),
            caller,
            "deposit must seed keychain tx_origin = deposit caller"
        );
    }

    /// A system deposit (L1-info / predeploy upgrade tick) must also flow through the
    /// seeding path without faulting. The seed is inert there (system deposits never read
    /// the keychain) — `tx_origin`/`transaction_key` are transient (TSTORE), so this does
    /// not perturb the state root. See `sci/docs/plan-a-l1-escape-hatch.md` §5.1.
    #[test]
    fn system_deposit_seeds_without_faulting() {
        let caller = Address::from([0x22; 20]);
        assert_eq!(deposit_seeded_tx_origin(caller, true), caller);
    }
}
