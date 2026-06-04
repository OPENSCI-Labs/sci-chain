// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAgentCircuitBreaker } from "../interfaces/IAgentCircuitBreaker.sol";
import { ISciAgentState } from "../interfaces/ISciAgentState.sol";

import { Ownable } from "@openzeppelin/contracts/access/Ownable.sol";

/// @title  AgentCircuitBreaker
/// @notice Predeploy at 0xBBBB..0003. Admin-gated facade around the `SciAgentState`
///         precompile's `tripKey/untripKey`. The precompile permits exactly this
///         contract's address as caller for those mutators; the contract enforces a
///         two-tier access model (owner + arbitrary guardians) on top.
///
/// @dev    Plan A enforcement: a trip set here is checked by the pre-execution
///         keychain hook (`run_aa_keychain_hook`) for every AA tx (type `0x76`) —
///         a tripped session key's batch is rejected before execution, in addition
///         to the legacy Plan B hook path. Trip/untrip state lives in the
///         `SciAgentState` precompile (indexed by session key), so the gate applies
///         uniformly across both tx paths.
contract AgentCircuitBreaker is IAgentCircuitBreaker, Ownable {
    /// Address of the `SciAgentState` precompile.
    address internal constant SCI_AGENT_STATE = 0xAaAAAaAA00000000000000000000000000000001;

    mapping(address => bool) private _guardians;

    modifier onlyGuardian() {
        if (msg.sender != owner() && !_guardians[msg.sender]) revert UnauthorizedGuardian();
        _;
    }

    constructor(address initialOwner) Ownable(initialOwner) { }

    function trip(address sessionKey, bytes32 reason) external onlyGuardian {
        ISciAgentState(SCI_AGENT_STATE).tripKey(sessionKey);
        emit Tripped(sessionKey, msg.sender, reason);
    }

    function untrip(address sessionKey) external onlyGuardian {
        ISciAgentState(SCI_AGENT_STATE).untripKey(sessionKey);
        emit Untripped(sessionKey, msg.sender);
    }

    function isTripped(address sessionKey) external view returns (bool) {
        return ISciAgentState(SCI_AGENT_STATE).isTripped(sessionKey);
    }

    function isGuardian(address account) external view returns (bool) {
        return account == owner() || _guardians[account];
    }

    function setGuardian(address guardian, bool authorized) external onlyOwner {
        if (guardian == address(0)) revert ZeroAddress();
        _guardians[guardian] = authorized;
        emit GuardianUpdated(guardian, authorized);
    }
}
