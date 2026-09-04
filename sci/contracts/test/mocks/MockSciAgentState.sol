// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { ISciAgentState } from "../../src/interfaces/ISciAgentState.sol";

/// @notice Forge-side stand-in for the SciAgentState precompile. Mutators check that
///         `msg.sender == AGENT_CIRCUIT_BREAKER_ADDRESS`, mirroring the real precompile.
contract MockSciAgentState is ISciAgentState {
    address public constant AGENT_CIRCUIT_BREAKER = 0xBbBbbBbB00000000000000000000000000000003;

    mapping(address => bool) private _tripped;

    function tripKey(address sessionKey) external {
        if (msg.sender != AGENT_CIRCUIT_BREAKER) revert Unauthorized();
        _tripped[sessionKey] = true;
        emit TripStateUpdate(sessionKey, true);
    }

    function untripKey(address sessionKey) external {
        if (msg.sender != AGENT_CIRCUIT_BREAKER) revert Unauthorized();
        _tripped[sessionKey] = false;
        emit TripStateUpdate(sessionKey, false);
    }

    function isTripped(address sessionKey) external view returns (bool) {
        return _tripped[sessionKey];
    }
}
