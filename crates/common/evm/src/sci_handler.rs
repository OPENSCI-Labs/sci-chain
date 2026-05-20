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
    context_interface::{
        ContextTr, JournalTr, Transaction,
        result::{ExecutionResult, FromStringError},
    },
    handler::{
        EthFrame, EvmTr, FrameResult, Handler,
        evm::FrameTr,
        handler::EvmTrError,
    },
    inspector::{InspectorEvmTr, InspectorHandler},
    interpreter::interpreter::EthInterpreter,
    interpreter::interpreter_action::FrameInit,
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
