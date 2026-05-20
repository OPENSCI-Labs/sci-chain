//! SCI-specific extensions to [`AccountKeychain`].
//!
//! Tempo source files (`mod.rs`, `dispatch.rs`) stay verbatim per CLAUDE.md Rule #4 so
//! upstream syncs can `cp` files in unmodified. Anything SCI-only goes here; the only
//! Tempo-source edit is the single `mod sci_ext;` line at the bottom of `mod.rs`
//! (documented in CLAUDE.md's Upstream Tempo Sync section).
//!
//! Currently exposes [`AccountKeychain::key_is_active`], a public wrapper around the
//! crate-private [`AccountKeychain::load_active_key`] used by the pre-execution hook to
//! decide whether `tx.from` is a registered access key for `tx.to`.
use super::AccountKeychain;
use crate::{error::Result, storage::Handler};
use alloy_primitives::Address;

impl AccountKeychain {
    /// Returns `true` iff `keys[account][key_id]` is registered, not revoked, and not
    /// expired at the current block timestamp.
    ///
    /// System errors (database failures, OOG, panics) bubble up; "key not active"
    /// flavors (`KeyNotFound`, `KeyAlreadyRevoked`, `KeyExpired`) collapse to `Ok(false)`.
    pub fn key_is_active(&self, account: Address, key_id: Address) -> Result<bool> {
        let now = self.storage.timestamp().saturating_to::<u64>();
        match self.load_active_key(account, key_id, now) {
            Ok(_) => Ok(true),
            Err(err) if err.is_system_error() => Err(err),
            Err(_) => Ok(false),
        }
    }

    /// Reads the transient `transaction_key` slot raw — used by the pre-execution hook
    /// to carry the session-key identity across handler methods (set in
    /// `validate_against_state_and_deduct_caller`, read in `execution_result` to apply
    /// deferred spending-limit deductions). Returns `Address::ZERO` when no agent tx
    /// is active.
    pub fn transaction_key_raw(&self) -> Result<Address> {
        self.transaction_key.t_read()
    }
}
