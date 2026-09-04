//! SCI Chain precompile ABI bindings.
//!
//! Crate is named `sci-precompile-abi` but exposed to dependents via Cargo
//! `package = ...` rename as `tempo-contracts`, so verbatim Tempo source can call
//! `use tempo_contracts::precompiles::*;` without identifier substitution. See the
//! workspace `Cargo.toml` for the rename.

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
