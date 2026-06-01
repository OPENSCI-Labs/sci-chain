// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { AgentBudgetController } from "../src/agent/AgentBudgetController.sol";
import { IAgentBudgetController } from "../src/interfaces/IAgentBudgetController.sol";
import { MockAccountKeychain } from "./mocks/MockAccountKeychain.sol";

contract AgentBudgetControllerTest is Test {
    address constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;
    address constant TOKEN = address(0x20C0);

    AgentBudgetController budget;
    address rootAccount;
    address sessionKey;

    function setUp() public {
        MockAccountKeychain mock = new MockAccountKeychain();
        vm.etch(KEYCHAIN, address(mock).code);

        budget = new AgentBudgetController();
        rootAccount = makeAddr("rootAccount");
        sessionKey = makeAddr("sessionKey");

        MockAccountKeychain(KEYCHAIN).setRemainingLimit(rootAccount, sessionKey, TOKEN, 100 ether, 0);
    }

    function test_RemainingMatchesKeychain() public view {
        (uint256 amt, uint64 periodEnd) = budget.remaining(rootAccount, sessionKey, TOKEN);
        assertEq(amt, 100 ether);
        assertEq(periodEnd, 0);
    }

    function test_SetAndGetThreshold() public {
        vm.prank(rootAccount);
        budget.setThreshold(sessionKey, TOKEN, 10 ether);
        assertEq(budget.getThreshold(rootAccount, sessionKey, TOKEN), 10 ether);
    }

    function test_AlertEmittedBelowThreshold() public {
        vm.prank(rootAccount);
        budget.setThreshold(sessionKey, TOKEN, 200 ether);

        vm.expectEmit(true, true, true, true);
        emit IAgentBudgetController.BudgetAlert(rootAccount, sessionKey, TOKEN, 100 ether, 200 ether);

        (,, bool alerted) = budget.checkAndAlert(rootAccount, sessionKey, TOKEN);
        assertTrue(alerted);
    }

    function test_NoAlertAboveThreshold() public {
        vm.prank(rootAccount);
        budget.setThreshold(sessionKey, TOKEN, 50 ether);

        (,, bool alerted) = budget.checkAndAlert(rootAccount, sessionKey, TOKEN);
        assertFalse(alerted);
    }

    function test_NoAlertWhenUnconfigured() public {
        (,, bool alerted) = budget.checkAndAlert(rootAccount, sessionKey, TOKEN);
        assertFalse(alerted);
    }
}
