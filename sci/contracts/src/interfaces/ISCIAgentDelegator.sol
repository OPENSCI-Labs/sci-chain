// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  ISCIAgentDelegator
/// @notice EIP-7702 batch executor at 0xCCCC..0001. The pre-execution hook decodes
///         `execute(Call[])` calldata before any per-call execution. The `Call`
///         tuple shape is load-bearing — must match
///         `sci/crates/precompile-abi/src/predeploys/sci_agent_delegator.rs`.
interface ISCIAgentDelegator {
    struct Call {
        address target;
        uint256 value;
        bytes data;
    }

    event AgentBatchExecuted(
        address indexed account, address indexed sessionKey, uint256 callCount
    );
    event AgentCallExecuted(
        address indexed account, uint256 indexed index, address indexed target, uint256 value
    );

    error MissingTransactionKey();
    error CallReverted(uint256 index, bytes returnData);

    function execute(Call[] calldata calls) external payable;
}
