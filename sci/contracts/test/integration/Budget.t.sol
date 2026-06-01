// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Vm } from "forge-std/Vm.sol";

import { DevnetBase } from "./base/DevnetBase.sol";

import { AgentBudgetController } from "../../src/agent/AgentBudgetController.sol";
import { IAgentBudgetController } from "../../src/interfaces/IAgentBudgetController.sol";
import { MockAccountKeychain } from "../mocks/MockAccountKeychain.sol";

/// @title  BudgetIntegrationTest
/// @notice Exercises the deployed AgentBudgetController bytecode. The keychain
///         remaining-limit lookups are served by the mock (etched at setUp),
///         so we control "remaining" via setRemainingLimit and assert the
///         alert logic against it.
contract BudgetIntegrationTest is DevnetBase {
    address internal constant TOKEN = address(0x20C0000000000000000000000000000000000001);

    function _seedRemaining(address account, address keyId, address token, uint256 amount) internal {
        MockAccountKeychain(KEYCHAIN).setRemainingLimit(account, keyId, token, amount, 0);
    }

    function test_Remaining_ProxiesKeychain() public {
        _seedRemaining(ALICE, BOB, TOKEN, 1_000_000);
        (uint256 amt, uint64 periodEnd) =
            AgentBudgetController(BUDGET).remaining(ALICE, BOB, TOKEN);
        assertEq(amt, 1_000_000);
        assertEq(periodEnd, 0);
    }

    function test_SetAndGetThreshold() public {
        vm.prank(ALICE);
        AgentBudgetController(BUDGET).setThreshold(BOB, TOKEN, 100);
        assertEq(AgentBudgetController(BUDGET).getThreshold(ALICE, BOB, TOKEN), 100);
    }

    function test_CheckAndAlert_NoAlert_WhenRemainingAboveThreshold() public {
        _seedRemaining(ALICE, BOB, TOKEN, 1_000_000);
        vm.prank(ALICE);
        AgentBudgetController(BUDGET).setThreshold(BOB, TOKEN, 100);

        vm.recordLogs();
        vm.prank(ALICE);
        (, , bool alerted) = AgentBudgetController(BUDGET).checkAndAlert(ALICE, BOB, TOKEN);
        assertFalse(alerted);
        // Confirm no BudgetAlert event was emitted.
        Vm.Log[] memory entries = vm.getRecordedLogs();
        for (uint256 i; i < entries.length; ++i) {
            assertTrue(
                entries[i].topics[0] != keccak256(
                    "BudgetAlert(address,address,address,uint256,uint256)"
                ),
                "unexpected BudgetAlert"
            );
        }
    }

    function test_CheckAndAlert_EmitsAlert_WhenRemainingAtOrBelowThreshold() public {
        _seedRemaining(ALICE, BOB, TOKEN, 50);
        vm.prank(ALICE);
        AgentBudgetController(BUDGET).setThreshold(BOB, TOKEN, 100);

        vm.expectEmit(true, true, true, true);
        emit IAgentBudgetController.BudgetAlert(ALICE, BOB, TOKEN, 50, 100);

        vm.prank(ALICE);
        (uint256 remaining,, bool alerted) =
            AgentBudgetController(BUDGET).checkAndAlert(ALICE, BOB, TOKEN);
        assertTrue(alerted);
        assertEq(remaining, 50);
    }

    function test_CheckAndAlert_NoAlert_WhenThresholdZero() public {
        // Default threshold (uninitialized) is zero — should never alert,
        // regardless of remaining.
        _seedRemaining(ALICE, BOB, TOKEN, 0);
        vm.prank(ALICE);
        (,, bool alerted) = AgentBudgetController(BUDGET).checkAndAlert(ALICE, BOB, TOKEN);
        assertFalse(alerted);
    }

    function test_SetThreshold_PerAccountIsolation() public {
        vm.prank(ALICE);
        AgentBudgetController(BUDGET).setThreshold(BOB, TOKEN, 100);
        vm.prank(CHARLIE);
        AgentBudgetController(BUDGET).setThreshold(BOB, TOKEN, 999);

        assertEq(AgentBudgetController(BUDGET).getThreshold(ALICE, BOB, TOKEN), 100);
        assertEq(AgentBudgetController(BUDGET).getThreshold(CHARLIE, BOB, TOKEN), 999);
    }

    // -------- Fuzz tests --------

    /// Setting and reading a threshold roundtrips exactly for any (keyId, token,
    /// amount) tuple.
    function testFuzz_ThresholdRoundtrip(address keyId, address token, uint256 amount) public {
        vm.prank(ALICE);
        AgentBudgetController(BUDGET).setThreshold(keyId, token, amount);
        assertEq(AgentBudgetController(BUDGET).getThreshold(ALICE, keyId, token), amount);
    }

    /// Alert fires iff (threshold != 0 && remaining <= threshold). Fuzz both
    /// dimensions.
    function testFuzz_AlertEdge(uint256 threshold, uint256 remaining) public {
        _seedRemaining(ALICE, BOB, TOKEN, remaining);
        vm.prank(ALICE);
        AgentBudgetController(BUDGET).setThreshold(BOB, TOKEN, threshold);

        vm.prank(ALICE);
        (,, bool alerted) = AgentBudgetController(BUDGET).checkAndAlert(ALICE, BOB, TOKEN);

        bool expected = (threshold != 0) && (remaining <= threshold);
        assertEq(alerted, expected);
    }
}
