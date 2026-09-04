// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  ISciAgentState
/// @notice ABI for the SciAgentState precompile at 0xAAAA..0001. Holds SCI-only
///         protocol state (currently the CircuitBreaker trip flag per session key).
///         Mutators require `msg.sender == AGENT_CIRCUIT_BREAKER_ADDRESS`. Mirror of
///         `sci/crates/precompile-abi/src/precompiles/sci_agent_state.rs`.
interface ISciAgentState {
    event TripStateUpdate(address indexed sessionKey, bool isTripped);

    error Unauthorized();

    function tripKey(address sessionKey) external;

    function untripKey(address sessionKey) external;

    function isTripped(address sessionKey) external view returns (bool);
}
