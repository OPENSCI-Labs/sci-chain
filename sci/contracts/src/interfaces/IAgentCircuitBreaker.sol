// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  IAgentCircuitBreaker
/// @notice Predeploy at 0xBBBB..0003. Admin-gated facade around the trip state held
///         in the `SciAgentState` precompile (`0xAAAA..0001`). The pre-execution
///         hook reads trip status directly from the precompile; this contract is
///         the only address authorized to flip those flags.
interface IAgentCircuitBreaker {
    event Tripped(address indexed sessionKey, address indexed by, bytes32 reason);
    event Untripped(address indexed sessionKey, address indexed by);
    event GuardianUpdated(address indexed guardian, bool authorized);

    error UnauthorizedGuardian();
    error AlreadyAdmin();
    error ZeroAddress();

    function trip(address sessionKey, bytes32 reason) external;

    function untrip(address sessionKey) external;

    function isTripped(address sessionKey) external view returns (bool);

    function isGuardian(address account) external view returns (bool);

    function setGuardian(address guardian, bool authorized) external;
}
