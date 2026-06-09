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
//!
//! # Keychain semantic notes for SCI integrators
//!
//! These notes capture behaviors of the verbatim Tempo source that aren't obvious from
//! the function signatures and can trip up Solidity authors and SDK builders. We
//! document them SCI-side because `mod.rs` is `cp`-overwritten by upstream syncs.
//!
//! ## `remove_allowed_calls` leaves `is_scoped = true`
//!
//! [`AccountKeychain::remove_allowed_calls`] deletes a target from the scope's
//! `targets` set but does **not** flip `is_scoped` back to `false`. After removing the
//! last allowed target, the key is in `is_scoped = true && targets.is_empty()` — a
//! deliberate "scoped deny-all" state, not "unrestricted". Subsequent
//! [`AccountKeychain::set_allowed_calls`] calls keep the same scoped mode and add new
//! targets correctly. Callers wanting "return key to unrestricted mode" must
//! re-authorize the key (which resets the scope via the authorize path; see
//! `setKeyCallScopes(_, isScoped = false, _)` in the ABI).
//!
//! ## `get_key` reports `isRevoked = false` for non-existent keys
//!
//! [`AccountKeychain::get_key`] folds both "key was never registered" and "key has
//! been revoked" into a single `expiry == 0` short-circuit path. The returned
//! `KeyInfo.isRevoked` reflects the stored `is_revoked` flag, which is `false` by
//! default for never-registered keys. Callers cannot distinguish "missing" from
//! "registered-and-not-revoked-yet-but-with-zero-expiry" via `isRevoked` alone — they
//! must check that `expiry > 0` to confirm a key exists. The keychain's other getters
//! ([`AccountKeychain::get_allowed_calls`],
//! [`AccountKeychain::get_remaining_limit_with_period`]) deliberately collapse
//! missing/revoked/expired keys onto the same "deny-all / zero quota" return shape, so
//! consumers should treat them uniformly rather than branching on `isRevoked`.
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

    /// Reads the transient `tx_origin` slot raw — the symmetric reader for
    /// [`AccountKeychain::set_tx_origin`]. The handler seeds this in
    /// `validate_against_state_and_deduct_caller` (for normal txs, AA agent txs, and —
    /// per the L1 escape hatch, Tier 2 — deposit txs); the keychain admin gate
    /// `ensure_account_caller` reads it to require `tx_origin == msg_sender`. Returns
    /// `Address::ZERO` when no origin has been seeded for the current transaction.
    pub fn tx_origin_raw(&self) -> Result<Address> {
        self.tx_origin.t_read()
    }
}
