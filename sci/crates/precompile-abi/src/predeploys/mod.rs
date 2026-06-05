//! SCI Chain predeploy contract ABI bindings.
//!
//! Distinct from `precompiles/`, which is reserved for actual precompile ABIs. These ABI
//! definitions are the canonical source of truth shared between the Rust pre-execution
//! hook (which decodes calldata to validate per-call scope and spending limits) and
//! Solidity implementations — e.g. the ERC-20 / SCI-20 token selectors the keychain meters.

mod erc20;
pub use erc20::{IERC20, ISCI20};
