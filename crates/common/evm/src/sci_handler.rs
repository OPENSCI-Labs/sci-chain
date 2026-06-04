//! Handler wrapper that inserts the SCI Chain pre-execution hook before tx execution.
//!
//! [`SciHandler`] wraps [`BaseHandler`] and delegates every method to it verbatim except
//! [`Handler::validate_against_state_and_deduct_caller`], which — after the inner
//! handler does the usual gas pre-pay / nonce bump — calls
//! [`sci_precompiles::run_pre_execution_hook`] to enforce CircuitBreaker, call scope,
//! and spending-limit constraints for 7702-delegated agent txs. Deposit (system) txs
//! short-circuit so OP-Stack predeploy ticks aren't subjected to keychain checks.
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
    handler::{
        EthFrame, EvmTr, FrameResult, Handler,
        evm::FrameTr,
        handler::EvmTrError,
    },
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
    AaCall, HookOutcome, apply_aa_post_execution_deductions, apply_post_execution_deductions,
    run_aa_keychain_hook, run_pre_execution_hook,
};

use crate::{
    L1BlockInfo, BaseHaltReason, BaseSpecId,
    handler::{IsTxError, BaseHandler},
    transaction::{AaTransactionParts, DEPOSIT_TRANSACTION_TYPE, BaseTransactionError, BaseTxTr},
};

/// SCI Chain handler wrapping Base's [`BaseHandler`]. See module docs.
#[derive(Debug, Clone, Default)]
pub struct SciHandler<EVM, ERROR, FRAME> {
    inner: BaseHandler<EVM, ERROR, FRAME>,
}

impl<EVM, ERROR, FRAME> SciHandler<EVM, ERROR, FRAME> {
    /// Creates a fresh [`SciHandler`] wrapping a fresh [`BaseHandler`].
    pub fn new() -> Self {
        Self {
            inner: BaseHandler::new(),
        }
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
                Journal: JournalTr<State = EvmState, Database: alloy_evm::Database> + core::fmt::Debug,
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
                Journal: JournalTr<State = EvmState, Database: alloy_evm::Database> + core::fmt::Debug,
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
                let signer_before =
                    evm.ctx().journal_mut().load_account(signer)?.data.info.balance;
                if let Some(err) = evm.ctx().journal_mut().transfer(fp, signer, prefund)? {
                    return Err(ERROR::from_string(format!(
                        "fee_payer {fp:?} cannot cover gas: {err:?}"
                    )));
                }
                self.inner.validate_against_state_and_deduct_caller(evm)?;
                let signer_after =
                    evm.ctx().journal_mut().load_account(signer)?.data.info.balance;
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

        // Deposit (system) txs invoke predeploy state ticks and must bypass the SCI
        // keychain hook — they aren't agent txs and the keychain isn't relevant.
        if evm.ctx().tx().tx_type() == DEPOSIT_TRANSACTION_TYPE {
            return Ok(());
        }

        // Plan A 2c — AA agent-tx authorization. An AA tx with `root` set acts on behalf
        // of `root`, so the keychain must authorize the session key (`keys[root][signer]`)
        // and gate the batch (circuit breaker + per-call scope). Extract the batch here
        // (the hook crate can't read the AA tx env) and run the AA-native keychain hook.
        // AA txs without `root` (a plain batch executed as the signer) and every non-AA tx
        // fall through to the legacy Plan B hook, which no-ops for non-agent traffic.
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
                    evm, root_addr, signer, &calls, gas_reservation,
                )? {
                    HookOutcome::Pass => Ok(()),
                    HookOutcome::Reject(err) => Err(err),
                }
            }
            _ => match run_pre_execution_hook::<EVM, ERROR>(evm)? {
                HookOutcome::Pass => Ok(()),
                HookOutcome::Reject(err) => Err(err),
            },
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
        // Q4 strong-R1: apply the deferred spending-limit deductions only if the EVM
        // body completed successfully. The pre-execution hook does a pre-flight check
        // but doesn't write; this is where the real deduction lands, so a tx that the
        // hook authorized but the body then reverts costs zero quota.
        //
        // Deposit txs and non-agent txs short-circuit inside the helper (it reads the
        // keychain's transient `transaction_key` slot and returns immediately on zero).
        if frame_result.interpreter_result().result.is_ok() {
            // AA agent txs (root set) meter against the keychain via the AA-native path
            // (native value + gas → address(0) sentinel; ERC-20 → per-token); every other
            // tx (incl. AA without root) uses the legacy Plan B deduction, which no-ops when
            // the keychain transient `transaction_key` slot is zero.
            let root = evm.ctx().tx().aa_parts().and_then(|a| a.root);
            if let Some(root_addr) = root {
                let signer = evm.ctx().tx().caller();
                let raw_fee_payer = evm.ctx().tx().aa_parts().and_then(|a| a.fee_payer);
                let calls: Vec<AaCall> = evm
                    .ctx()
                    .tx()
                    .aa_parts()
                    .map(|parts| {
                        parts
                            .calls
                            .iter()
                            .map(|c| AaCall { to: c.to, value: c.value, input: c.input.to_vec() })
                            .collect()
                    })
                    .unwrap_or_default();
                // D-gas: the gas the agent actually spent (gas_used * max_fee, pessimistic,
                // matching the pre-flight reservation), counted only when root sponsored gas.
                let gas_deduction = if raw_fee_payer == Some(root_addr) {
                    let gas_used = frame_result.gas().used();
                    let max_fee = evm.ctx().tx().max_fee_per_gas();
                    U256::from(gas_used).saturating_mul(U256::from(max_fee))
                } else {
                    U256::ZERO
                };
                apply_aa_post_execution_deductions::<EVM, ERROR>(
                    evm, root_addr, signer, &calls, gas_deduction,
                )?;
            } else {
                apply_post_execution_deductions::<EVM, ERROR>(evm)?;
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
                Journal: JournalTr<State = EvmState, Database: alloy_evm::Database> + core::fmt::Debug,
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
        if let Some(aa) =
            evm.ctx().tx().aa_parts().cloned().filter(|aa| !aa.calls.is_empty())
        {
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
