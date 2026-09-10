//! SCI Chain precompile ABI bindings.
//!
//! Crate is named `sci-precompile-abi` but exposed to dependents via Cargo
//! `package = ...` rename as `tempo-contracts`, so ported Tempo source can call
//! `use tempo_contracts::precompiles::*;` without identifier substitution. See the
//! workspace `Cargo.toml` for the rename.
//!
//! The ABI definitions (notably `account_keychain`) are adapted from
//! tempoxyz/tempo v1.7.1 (MIT OR Apache-2.0, Copyright (c) 2025 Tempo
//! Contributors) — see `THIRD_PARTY_NOTICES.md` at the repository root.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]
#![cfg_attr(docsrs, feature(doc_cfg))]

/// Helper macro mirroring Tempo's `tempo_contracts::sol!` — optionally derives serde
/// traits behind a feature flag. Ported source files call `crate::sol!` against this.
macro_rules! sol {
    ($($input:tt)*) => {
        #[cfg(feature = "serde")]
        alloy_sol_types::sol! {
            #[derive(serde::Serialize, serde::Deserialize)]
            $($input)*
        }
        #[cfg(not(feature = "serde"))]
        alloy_sol_types::sol! {
            $($input)*
        }
    };
}

pub(crate) use sol;

pub mod precompiles;
pub mod predeploys;
