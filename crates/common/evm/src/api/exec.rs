//! Base-specific [`BaseContextTr`] trait alias and [`BaseError`] type alias.
use revm::{
    context_interface::{Cfg, ContextTr, Database, JournalTr, result::EVMError},
    state::EvmState,
};

use crate::{BaseSpecId, BaseTransactionError, L1BlockInfo, transaction::BaseTxTr};

/// Trait alias for the context type required by [`BaseEvm`][crate::BaseEvm].
///
/// Satisfied by [`crate::BaseContext`] for any database, binding the transaction type to
/// [`BaseTxTr`], the spec to [`BaseSpecId`], and the chain extension to [`L1BlockInfo`].
///
/// **SCI patch**: the `Db: alloy_evm::Database` and `Journal: ... + core::fmt::Debug`
/// bounds were added on the SCI fork. They are required by `SciHandler`'s pre-execution
/// hook, which constructs an `alloy_evm::EvmInternals` (whose `new` constructor needs the
/// journal to be Debug). All concrete `BaseContext<DB>` instances Base actually uses
/// (`State<...>`, `InMemoryDB`, `EmptyDB`) satisfy these, so adding the bounds here is
/// non-breaking for upstream Base callers in practice.
pub trait BaseContextTr:
    ContextTr<
        Db: alloy_evm::Database,
        Journal: JournalTr<State = EvmState, Database: alloy_evm::Database> + core::fmt::Debug,
        Tx: BaseTxTr,
        Cfg: Cfg<Spec = BaseSpecId>,
        Chain = L1BlockInfo,
    >
{
}

impl<T> BaseContextTr for T where
    T: ContextTr<
            Db: alloy_evm::Database,
            Journal: JournalTr<State = EvmState, Database: alloy_evm::Database> + core::fmt::Debug,
            Tx: BaseTxTr,
            Cfg: Cfg<Spec = BaseSpecId>,
            Chain = L1BlockInfo,
        >
{
}

/// Error type for [`BaseEvm`][crate::BaseEvm] execution, parameterized over the database
/// error type [`DB`].
pub type BaseError<DB> = EVMError<<DB as Database>::Error, BaseTransactionError>;

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use revm::{
        ExecuteEvm, SystemCallEvm,
        database::{InMemoryDB, State},
    };

    use crate::{BaseContext, Builder, DefaultBase};

    /// Verifies that the system call caller is loaded into the EVM state cache so it appears in the
    /// execution witness.
    ///
    /// The state cache (`State.cache.accounts`) is exactly what `ExecutionWitnessRecord` reads to
    /// build the `hashed_state` fed to `state_provider.witness(...)`. Without the
    /// `load_account_with_code_mut` call in `system_call_one_with_caller`, the caller account
    /// would not be cached and would be absent from the generated witness, breaking Geth proof
    /// compatibility.
    ///
    /// See: <https://github.com/bluealloy/revm/issues/3484>
    #[test]
    fn system_call_caller_appears_in_witness() {
        let caller = Address::repeat_byte(0xCA);
        let contract = Address::repeat_byte(0xAB);

        // Use State with bundle tracking, mirroring the witness generation path in
        // Builder::witness and debug_executionWitness.
        let state =
            State::builder().with_database(InMemoryDB::default()).with_bundle_update().build();

        let ctx = BaseContext::base().with_db(state);
        let mut evm = ctx.build_base();

        // Execute a system call. This internally calls `load_account_with_code_mut(caller)`,
        // causing the State DB to load and cache the caller's account in `State.cache.accounts`.
        let _ = evm.system_call_one_with_caller(caller, contract, Default::default());

        // Finalize to flush the journal, then inspect the underlying State cache.
        // `ExecutionWitnessRecord::from_executed_state` iterates `State.cache.accounts` to build
        // the hashed state, so the caller must appear here to be included in the witness.
        let _ = evm.finalize();
        let state = evm.into_context().journaled_state.database;

        assert!(
            state.cache.accounts.contains_key(&caller),
            "system call caller must be in state cache for Geth proof compatibility"
        );
    }
}
