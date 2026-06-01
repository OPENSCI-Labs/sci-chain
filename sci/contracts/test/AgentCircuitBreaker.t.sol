// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { AgentCircuitBreaker } from "../src/agent/AgentCircuitBreaker.sol";
import { IAgentCircuitBreaker } from "../src/interfaces/IAgentCircuitBreaker.sol";
import { MockSciAgentState } from "./mocks/MockSciAgentState.sol";

contract AgentCircuitBreakerTest is Test {
    address constant SCI_AGENT_STATE = 0xAaAAAaAA00000000000000000000000000000001;
    address constant SESSION_KEY = address(0xC0DE);

    AgentCircuitBreaker cb;
    address owner;
    address guardian;
    address attacker;

    function setUp() public {
        owner = makeAddr("owner");
        guardian = makeAddr("guardian");
        attacker = makeAddr("attacker");

        // The mock checks `msg.sender == AgentCircuitBreaker predeploy address`, so the
        // CB must live at exactly that constant. Deploy normally then `vm.etch` it into
        // place.
        MockSciAgentState mock = new MockSciAgentState();
        vm.etch(SCI_AGENT_STATE, address(mock).code);

        AgentCircuitBreaker tmp = new AgentCircuitBreaker(owner);
        bytes memory code = address(tmp).code;
        address predeploy = 0xBbBbbBbB00000000000000000000000000000003;
        vm.etch(predeploy, code);
        // Initialize Ownable storage at the predeploy. With OZ v5 `Ownable._owner` is
        // slot 0; we set it directly to `owner`.
        vm.store(predeploy, bytes32(uint256(0)), bytes32(uint256(uint160(owner))));
        cb = AgentCircuitBreaker(predeploy);
    }

    function test_OwnerCanTrip() public {
        vm.prank(owner);
        cb.trip(SESSION_KEY, bytes32("manual-trip"));
        assertTrue(cb.isTripped(SESSION_KEY));
    }

    function test_GuardianCanTrip() public {
        vm.prank(owner);
        cb.setGuardian(guardian, true);

        vm.prank(guardian);
        cb.trip(SESSION_KEY, bytes32(0));
        assertTrue(cb.isTripped(SESSION_KEY));
    }

    function test_RevertWhen_UnauthorizedCallerTrips() public {
        vm.prank(attacker);
        vm.expectRevert(IAgentCircuitBreaker.UnauthorizedGuardian.selector);
        cb.trip(SESSION_KEY, bytes32(0));
    }

    function test_Untrip() public {
        vm.prank(owner);
        cb.trip(SESSION_KEY, bytes32(0));
        assertTrue(cb.isTripped(SESSION_KEY));

        vm.prank(owner);
        cb.untrip(SESSION_KEY);
        assertFalse(cb.isTripped(SESSION_KEY));
    }
}
