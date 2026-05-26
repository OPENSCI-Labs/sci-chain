//! Test utilities shared by precompile unit tests.

use crate::{Precompile, error::Result, storage::hashmap::HashMapStorageProvider};
use alloy_primitives::{Address, U256};
use alloy_sol_types::SolError;
use revm::precompile::PrecompileError;
use tempo_contracts::precompiles::UnknownFunctionSelector;

/// **No-op stub** of Tempo's `tip20::TIP20Setup` helper, kept so that ported tests which
/// call `TIP20Setup::path_usd(account).apply()?` compile verbatim against the SCI source.
///
/// SCI Chain does not implement TIP-20 tokens (it uses standard ERC-20), so there is no
/// `pathUSD` predeploy to set up — `apply()` simply returns `Ok(())`. Tests that depend
/// on real TIP-20 state would need a different fixture; the ones currently using this
/// stub only call it as opaque setup and don't observe pathUSD storage state.
///
/// This shape lets `cp` from Tempo upstream merge cleanly into our keychain `mod.rs`
/// without rewriting test bodies.
pub struct TIP20Setup {
    _account: Address,
}

impl TIP20Setup {
    /// Tempo-compat: configure pathUSD for `admin`. No-op in SCI.
    pub fn path_usd(admin: Address) -> Self {
        Self { _account: admin }
    }

    /// Tempo-compat: apply the staged configuration. No-op in SCI.
    pub fn apply(self) -> Result<()> {
        Ok(())
    }
}

/// Creates a test [`HashMapStorageProvider`] (chain ID 1) paired with a random address.
pub fn setup_storage() -> (HashMapStorageProvider, Address) {
    (HashMapStorageProvider::new(1), Address::random())
}

/// Test helper for constructing EVM words from hex string literals.
///
/// Takes an array of hex strings (with or without "0x" prefix), concatenates them
/// left-to-right, left-pads with zeros to 32 bytes, and returns a U256.
pub fn gen_word_from(values: &[&str]) -> U256 {
    let mut bytes = Vec::new();

    for value in values {
        let hex_str = value.strip_prefix("0x").unwrap_or(value);
        assert!(hex_str.len() % 2 == 0, "Hex string '{value}' has odd length");

        for i in (0..hex_str.len()).step_by(2) {
            let byte_str = &hex_str[i..i + 2];
            let byte = u8::from_str_radix(byte_str, 16)
                .unwrap_or_else(|e| panic!("Invalid hex in '{value}': {e}"));
            bytes.push(byte);
        }
    }

    assert!(bytes.len() <= 32, "Total bytes ({}) exceed 32-byte slot limit", bytes.len());

    let mut slot_bytes = [0u8; 32];
    let start_idx = 32 - bytes.len();
    slot_bytes[start_idx..].copy_from_slice(&bytes);
    U256::from_be_bytes(slot_bytes)
}

/// Checks that all selectors in an interface have dispatch handlers.
///
/// Calls each selector with dummy parameters and checks for "Unknown function selector" errors.
/// Returns unsupported selectors as `(selector_bytes, function_name)` tuples.
pub fn check_selector_coverage<P: Precompile>(
    precompile: &mut P,
    selectors: &[[u8; 4]],
    interface_name: &str,
    name_lookup: impl Fn([u8; 4]) -> Option<&'static str>,
) -> Vec<([u8; 4], &'static str)> {
    let mut unsupported_selectors = Vec::new();

    for selector in selectors.iter() {
        let mut calldata = selector.to_vec();
        calldata.extend_from_slice(&[0u8; 32]);

        let result = precompile.call(&calldata, Address::ZERO);

        // Old-format unknown-selector: returned as PrecompileError::Other or PrecompileError::Fatal
        let is_unsupported_old = match &result {
            Err(PrecompileError::Other(msg)) => msg.contains("Unknown function selector"),
            Err(PrecompileError::Fatal(msg)) => msg.contains("Unknown function selector"),
            _ => false,
        };

        // New-format: reverted PrecompileOutput with an ABI-encoded UnknownFunctionSelector
        let is_unsupported_new = if let Ok(output) = &result {
            output.is_revert() && UnknownFunctionSelector::abi_decode(&output.bytes).is_ok()
        } else {
            false
        };

        if (is_unsupported_old || is_unsupported_new)
            && let Some(name) = name_lookup(*selector)
        {
            unsupported_selectors.push((*selector, name));
        }
    }

    if !unsupported_selectors.is_empty() {
        eprintln!("Unsupported {interface_name} selectors:");
        for (selector, name) in &unsupported_selectors {
            eprintln!("  - {name} ({selector:?})");
        }
    }

    unsupported_selectors
}

/// Asserts that multiple selector coverage checks all pass (no unsupported selectors).
pub fn assert_full_coverage(results: impl IntoIterator<Item = Vec<([u8; 4], &'static str)>>) {
    let all_unsupported: Vec<_> = results
        .into_iter()
        .flat_map(|r| r.into_iter())
        .map(|(_, name)| name)
        .collect();

    assert!(
        all_unsupported.is_empty(),
        "Found {} unsupported selectors: {:?}",
        all_unsupported.len(),
        all_unsupported
    );
}
