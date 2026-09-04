//! Contains the transaction trait abstraction.

use auto_impl::auto_impl;
use revm::{
    context_interface::transaction::Transaction,
    primitives::{B256, Bytes},
};

use crate::{AaTransactionParts, DEPOSIT_TRANSACTION_TYPE};

/// Base Transaction trait.
#[auto_impl(&, &mut, Box, Arc)]
pub trait BaseTxTr: Transaction {
    /// Enveloped transaction bytes.
    fn enveloped_tx(&self) -> Option<&Bytes>;

    /// SCI AA (account-abstraction, type `0x76`) transaction parts, if this env was built
    /// from a `BaseTxEnvelope::Aa`. `None` for every other tx type. Used by `SciHandler`
    /// to drive the multi-call executor.
    fn aa_parts(&self) -> Option<&AaTransactionParts> {
        None
    }

    /// Source hash of the deposit transaction.
    fn source_hash(&self) -> Option<B256>;

    /// Mint of the deposit transaction
    fn mint(&self) -> Option<u128>;

    /// Whether the transaction is a system transaction
    fn is_system_transaction(&self) -> bool;

    /// Returns `true` if transaction is of type [`DEPOSIT_TRANSACTION_TYPE`].
    fn is_deposit(&self) -> bool {
        self.tx_type() == DEPOSIT_TRANSACTION_TYPE
    }
}
