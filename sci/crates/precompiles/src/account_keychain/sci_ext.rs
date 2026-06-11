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
use super::{AccountKeychain, AuthorizedKey};
use crate::{
    error::Result,
    storage::{
        Handler, LayoutCtx, Storable, StorageCtx, StorageKey, hashmap::HashMapStorageProvider,
        packing::PackedSlot,
    },
};
use alloy_primitives::{Address, U256};

impl AccountKeychain {
    /// Computes the raw storage slot (within the keychain precompile's account storage)
    /// holding the packed [`AuthorizedKey`] word for `keys[account][key_id]`.
    ///
    /// Pure slot math (Solidity mapping layout, no storage context required) so hosts
    /// outside an EVM execution — e.g. the txpool validator's AA admission gate — can
    /// read the word straight from a `StateProvider` and decode it with
    /// [`AccountKeychain::decode_authorized_key`].
    pub fn authorized_key_slot(account: Address, key_id: Address) -> U256 {
        let kc = Self::default();
        key_id.mapping_slot(account.mapping_slot(kc.keys.slot()))
    }

    /// Decodes a raw storage word read from [`AccountKeychain::authorized_key_slot`]
    /// into an [`AuthorizedKey`], using the same `Storable` packing the precompile
    /// itself writes with. Like [`AccountKeychain::encode_authorized_key`], runs inside
    /// a throwaway storage context so it is callable from non-EVM hosts (the txpool).
    pub fn decode_authorized_key(word: U256) -> Result<AuthorizedKey> {
        let mut scratch = HashMapStorageProvider::new(1);
        StorageCtx::enter(&mut scratch, || {
            AuthorizedKey::load(&PackedSlot(word), U256::ZERO, LayoutCtx::FULL)
        })
    }

    /// Encodes an [`AuthorizedKey`] into the raw storage word the precompile would
    /// write at [`AccountKeychain::authorized_key_slot`] — the inverse of
    /// [`AccountKeychain::decode_authorized_key`]. Used by tests and genesis tooling to
    /// seed keychain state without an EVM execution context.
    ///
    /// The derived `Storable::store` consults the ambient [`StorageCtx`] spec for its
    /// slot-init strategy, so this enters a throwaway in-memory context around the pack;
    /// the encoded word itself is spec-independent (both strategies start from a zero
    /// word here).
    pub fn encode_authorized_key(key: &AuthorizedKey) -> Result<U256> {
        let mut scratch = HashMapStorageProvider::new(1);
        StorageCtx::enter(&mut scratch, || {
            let mut slot = PackedSlot(U256::ZERO);
            key.store(&mut slot, U256::ZERO, LayoutCtx::FULL)?;
            Ok(slot.0)
        })
    }

    /// Coarse admission-time check over a raw [`AccountKeychain::authorized_key_slot`]
    /// word: `true` iff some key is registered (non-zero expiry), not revoked, and not
    /// expired at `now`.
    ///
    /// This mirrors the *shape* of [`AccountKeychain::key_is_active`] without a storage
    /// context. It is deliberately advisory — the execution-time pre-execution hook
    /// remains the authoritative gate — and exists so the txpool can refuse obviously
    /// unauthorized sponsored AA txs (zero-cost pool stuffing, review finding M-3)
    /// before they occupy pool slots.
    pub fn authorized_key_word_is_active(word: U256, now: u64) -> bool {
        match Self::decode_authorized_key(word) {
            Ok(key) => key.expiry != 0 && !key.is_revoked && key.expiry > now,
            Err(_) => false,
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{PrecompileStorageProvider, StorageCtx, hashmap::HashMapStorageProvider};
    use tempo_contracts::precompiles::ACCOUNT_KEYCHAIN_ADDRESS;

    /// Anchors the out-of-EVM slot math + word decoding (used by the txpool AA admission
    /// gate, review finding M-3) to the precompile's own storage machinery: write an
    /// `AuthorizedKey` through the real handlers, then re-read it raw at
    /// [`AccountKeychain::authorized_key_slot`] and decode.
    #[test]
    fn authorized_key_slot_and_decode_match_storable_write() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let account = Address::repeat_byte(0x11);
        let key_id = Address::repeat_byte(0x22);
        let written = AuthorizedKey {
            signature_type: 1,
            expiry: 1_234_567,
            enforce_limits: true,
            is_revoked: false,
        };

        let w = written.clone();
        StorageCtx::enter(&mut storage, move || -> Result<()> {
            let mut kc = AccountKeychain::new();
            kc.keys[account][key_id].write(w)?;
            Ok(())
        })?;

        let slot = AccountKeychain::authorized_key_slot(account, key_id);
        let word = storage.sload(ACCOUNT_KEYCHAIN_ADDRESS, slot)?;
        assert_ne!(word, U256::ZERO, "raw word must land at the computed slot");

        let decoded = AccountKeychain::decode_authorized_key(word)?;
        assert_eq!(decoded, written);
        assert_eq!(
            AccountKeychain::encode_authorized_key(&written)?,
            word,
            "encode must produce the exact word the precompile writes"
        );

        assert!(AccountKeychain::authorized_key_word_is_active(word, 1_000_000));
        assert!(
            !AccountKeychain::authorized_key_word_is_active(word, 1_234_567),
            "expiry boundary: key expired exactly at `now` is inactive"
        );
        assert!(
            !AccountKeychain::authorized_key_word_is_active(U256::ZERO, 0),
            "never-registered key (zero word) is inactive"
        );
        Ok(())
    }

    /// A revoked key decodes as inactive regardless of expiry.
    #[test]
    fn revoked_key_word_is_inactive() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new(1);
        let account = Address::repeat_byte(0x33);
        let key_id = Address::repeat_byte(0x44);

        StorageCtx::enter(&mut storage, move || -> Result<()> {
            let mut kc = AccountKeychain::new();
            kc.keys[account][key_id].write(AuthorizedKey {
                signature_type: 0,
                expiry: u64::MAX,
                enforce_limits: false,
                is_revoked: true,
            })?;
            Ok(())
        })?;

        let slot = AccountKeychain::authorized_key_slot(account, key_id);
        let word = storage.sload(ACCOUNT_KEYCHAIN_ADDRESS, slot)?;
        assert!(!AccountKeychain::authorized_key_word_is_active(word, 0));
        Ok(())
    }
}
