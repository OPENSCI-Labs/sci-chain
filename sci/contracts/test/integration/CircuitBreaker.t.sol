// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { DevnetBase } from "./base/DevnetBase.sol";

import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";
import { IAgentCircuitBreaker } from "../../src/interfaces/IAgentCircuitBreaker.sol";
import { ISciAgentState } from "../../src/interfaces/ISciAgentState.sol";

/// @title  CircuitBreakerIntegrationTest
/// @notice Exercises the deployed AgentCircuitBreaker bytecode against a mocked
///         SciAgentState precompile. Covers concrete owner/guardian flows and
///         fuzz-explores arbitrary trip targets + guardian addresses.
contract CircuitBreakerIntegrationTest is DevnetBase {
    bytes32 internal constant REASON_MANUAL = bytes32("manual-trip");
    bytes32 internal constant REASON_NONE = bytes32(0);

    function _trip(address by, address target, bytes32 reason) internal {
        vm.prank(by);
        AgentCircuitBreaker(BREAKER).trip(target, reason);
    }

    function _untrip(address by, address target) internal {
        vm.prank(by);
        AgentCircuitBreaker(BREAKER).untrip(target);
    }

    function test_Owner_CanTrip_PrecompileMirrors() public {
        _trip(ALICE, BOB, REASON_MANUAL);

        assertTrue(AgentCircuitBreaker(BREAKER).isTripped(BOB));
        assertTrue(ISciAgentState(SCI_AGENT_STATE).isTripped(BOB));
    }

    function test_Owner_CanUntrip_PrecompileMirrors() public {
        _trip(ALICE, BOB, REASON_MANUAL);
        _untrip(ALICE, BOB);

        assertFalse(AgentCircuitBreaker(BREAKER).isTripped(BOB));
        assertFalse(ISciAgentState(SCI_AGENT_STATE).isTripped(BOB));
    }

    function test_Guardian_CanTrip() public {
        vm.prank(ALICE);
        AgentCircuitBreaker(BREAKER).setGuardian(CHARLIE, true);

        _trip(CHARLIE, BOB, REASON_NONE);
        assertTrue(AgentCircuitBreaker(BREAKER).isTripped(BOB));
    }

    function test_NonGuardian_CannotTrip() public {
        // Bob has never been granted guardian status.
        vm.prank(BOB);
        vm.expectRevert(IAgentCircuitBreaker.UnauthorizedGuardian.selector);
        AgentCircuitBreaker(BREAKER).trip(CHARLIE, REASON_NONE);
    }

    function test_SetGuardian_ZeroAddress_Reverts() public {
        vm.prank(ALICE);
        vm.expectRevert(IAgentCircuitBreaker.ZeroAddress.selector);
        AgentCircuitBreaker(BREAKER).setGuardian(address(0), true);
    }

    function test_SetGuardian_OnlyOwner() public {
        // Bob is not owner; should revert with OZ Ownable.
        vm.prank(BOB);
        vm.expectRevert();
        AgentCircuitBreaker(BREAKER).setGuardian(CHARLIE, true);
    }

    function test_PrecompileDirectMutator_Reverts() public {
        // Calling tripKey directly (not via the breaker contract) must fail —
        // the Mock enforces msg.sender == BREAKER, matching the Rust precompile.
        vm.prank(ALICE);
        vm.expectRevert(ISciAgentState.Unauthorized.selector);
        ISciAgentState(SCI_AGENT_STATE).tripKey(BOB);
    }

    function test_TripUntrip_IsIdempotent() public {
        // Tripping an already-tripped key is a no-op (state stays true).
        _trip(ALICE, BOB, REASON_MANUAL);
        _trip(ALICE, BOB, REASON_NONE);
        assertTrue(AgentCircuitBreaker(BREAKER).isTripped(BOB));

        _untrip(ALICE, BOB);
        _untrip(ALICE, BOB);
        assertFalse(AgentCircuitBreaker(BREAKER).isTripped(BOB));
    }

    // -------- Fuzz tests --------

    /// Tripping any non-zero session key by alice must always succeed and
    /// reflect in both views. We exclude the zero address only to keep the
    /// fuzzer focused; the contract has no special-case for it.
    function testFuzz_OwnerTrip_AnyTarget(address target) public {
        vm.assume(target != address(0));
        _trip(ALICE, target, REASON_NONE);
        assertTrue(AgentCircuitBreaker(BREAKER).isTripped(target));
        assertTrue(ISciAgentState(SCI_AGENT_STATE).isTripped(target));
    }

    /// Any non-{owner, guardian} caller must be rejected.
    function testFuzz_NonGuardian_AlwaysReverts(address attacker, address target) public {
        vm.assume(attacker != ALICE);
        vm.assume(attacker != address(0));
        // Don't grant the attacker guardian status.
        vm.assume(!AgentCircuitBreaker(BREAKER).isGuardian(attacker));

        vm.prank(attacker);
        vm.expectRevert(IAgentCircuitBreaker.UnauthorizedGuardian.selector);
        AgentCircuitBreaker(BREAKER).trip(target, REASON_NONE);
    }

    /// setGuardian followed by isGuardian must be consistent for any non-zero
    /// guardian. Owner is always a guardian regardless of the flag.
    function testFuzz_GuardianRoundtrip(address guardian, bool authorized) public {
        vm.assume(guardian != address(0));

        vm.prank(ALICE);
        AgentCircuitBreaker(BREAKER).setGuardian(guardian, authorized);

        bool expected = authorized || guardian == ALICE;
        assertEq(AgentCircuitBreaker(BREAKER).isGuardian(guardian), expected);
    }
}
