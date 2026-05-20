//! ABI for `SCIAgentDelegator` — the EIP-7702 batch executor predeploy.
//!
//! Agent flows: a session key signs a tx with `tx.to = root_account` (an EOA that has
//! 7702-delegated to `SCI_AGENT_DELEGATOR_ADDRESS`) and `tx.input = execute(Call[])`.
//! The Rust pre-execution hook decodes this calldata and validates each `Call` against
//! the keychain's scope and spending limits before any EVM execution begins.
//!
//! This ABI is the canonical shared source between the Rust hook and the Solidity
//! `SCIAgentDelegator.sol` contract (Heath's lane) — both must agree on the exact tuple
//! shape of `Call`.

use alloy_primitives::{Address, address};

crate::sol! {
    #[derive(Debug, PartialEq, Eq)]
    #[sol(abi)]
    interface ISCIAgentDelegator {
        /// One inner call inside an EIP-7702 batch.
        struct Call {
            address target;
            uint256 value;
            bytes data;
        }

        /// Batched call execution. Pre-execution hook decodes this calldata when an
        /// agent tx is detected (7702 delegation present + key registered).
        function execute(Call[] calldata calls) external;
    }
}

/// Address of the `SCIAgentDelegator` predeploy (EIP-7702 delegate target).
pub const SCI_AGENT_DELEGATOR_ADDRESS: Address =
    address!("0xCCCCCCCC00000000000000000000000000000001");
