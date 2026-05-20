//! SCI Chain predeploy contract ABI bindings.
//!
//! Distinct from `precompiles/`, which is reserved for actual precompile ABIs. Predeploys
//! are Solidity contracts deployed to fixed addresses at genesis (e.g. `SCIAgentDelegator`
//! at `0xCCCCCCCC...01`). The ABI definitions here are the canonical source of truth shared
//! between the Rust pre-execution hook (which decodes calldata to validate per-call scope
//! and spending limits) and Solidity implementations on Heath's branch.

mod sci_agent_delegator;
pub use sci_agent_delegator::{ISCIAgentDelegator, SCI_AGENT_DELEGATOR_ADDRESS};

mod erc20;
pub use erc20::{IERC20, ISCI20};
