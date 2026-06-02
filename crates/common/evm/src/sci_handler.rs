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

use revm::{
    bytecode::Bytecode,
    context_interface::{
        Cfg, ContextTr, JournalTr, LocalContextTr, Transaction,
        result::{ExecutionResult, FromStringError},
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
    primitives::TxKind,
    state::EvmState,
};

use sci_precompiles::{HookOutcome, apply_post_execution_deductions, run_pre_execution_hook};

use crate::{
    L1BlockInfo, BaseHaltReason, BaseSpecId,
    handler::{IsTxError, BaseHandler},
    transaction::{DEPOSIT_TRANSACTION_TYPE, BaseTransactionError, BaseTxTr},
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
        self.inner.validate_against_state_and_deduct_caller(evm)?;

        // Deposit (system) txs invoke predeploy state ticks and must bypass the SCI
        // keychain hook — they aren't agent txs and the keychain isn't relevant.
        if evm.ctx().tx().tx_type() == DEPOSIT_TRANSACTION_TYPE {
            return Ok(());
        }

        match run_pre_execution_hook::<EVM, ERROR>(evm)? {
            HookOutcome::Pass => Ok(()),
            HookOutcome::Reject(err) => Err(err),
        }
    }

    fn last_frame_result(
        &mut self,
        evm: &mut Self::Evm,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.last_frame_result(evm, frame_result)
    }

    /// Multi-call executor for SCI AA transactions (Plan A 2a).
    ///
    /// AA txs carry a batch (`aa` parts on the tx env); every other tx type has no `aa`
    /// parts and delegates to the standard single-call execution. The batch runs
    /// atomically: each [`Call`] executes as its own depth-0 frame with `msg.sender ==
    /// root` (the signer when no root is set), wrapped in one journal checkpoint so any
    /// call's failure rolls back the whole batch. Gas threads across calls; the returned
    /// [`FrameResult`] is the last call's, with gas normalized to the full tx limit.
    ///
    /// NOTE: this is the non-inspector consensus path. The inspector (tracing) path still
    /// falls back to single-call for AA — tracing parity is a follow-up.
    fn execution(
        &mut self,
        evm: &mut Self::Evm,
        init_and_floor_gas: &InitialAndFloorGas,
    ) -> Result<FrameResult, Self::Error> {
        let Some(aa) = evm.ctx().tx().aa_parts().cloned() else {
            return self.inner.execution(evm, init_and_floor_gas);
        };
        if aa.calls.is_empty() {
            // Defensive: txpool rejects empty batches; fall back rather than panic.
            return self.inner.execution(evm, init_and_floor_gas);
        }

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

            let frame_result = self.run_exec_loop(evm, frame_init)?;
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

    fn reimburse_caller(
        &self,
        evm: &mut Self::Evm,
        frame_result: &mut <<Self::Evm as EvmTr>::Frame as FrameTr>::FrameResult,
    ) -> Result<(), Self::Error> {
        self.inner.reimburse_caller(evm, frame_result)
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
            apply_post_execution_deductions::<EVM, ERROR>(evm)?;
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
}
