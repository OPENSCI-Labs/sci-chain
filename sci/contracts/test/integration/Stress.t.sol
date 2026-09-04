// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { DevnetBase } from "./base/DevnetBase.sol";

import { AgentAccessKeyRegistry } from "../../src/agent/AgentAccessKeyRegistry.sol";
import { AgentBudgetController } from "../../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";

/// @title  StressTest
/// @notice Heavy-load scenarios that exercise the predeploys with batch sizes
///         and iteration counts a manual operator would never type. Two goals:
///           1. Surface O(n²) or unbounded-loop bugs that pass at small N.
///           2. Provide a gas baseline for the per-contract operations so
///              future code changes can compare against `forge snapshot`.
contract StressTest is DevnetBase {
    /// Bind 100 keys in sequence. Catches:
    ///   - O(n) regressions if the registry ever added a linear search
    ///   - storage-collision bugs that only manifest at scale
    function test_Stress_BindManyKeys() public {
        uint256 n = 100;
        for (uint256 i; i < n; ++i) {
            address sk = address(uint160(uint256(keccak256(abi.encode("sk", i)))));
            bytes32 agentId = keccak256(abi.encode("agent", i));

            authorizeUnrestricted(ALICE, sk);

            vm.prank(ALICE);
            AgentAccessKeyRegistry(REGISTRY).bindKey(sk, agentId);

            assertTrue(AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, sk));
            assertEq(AgentAccessKeyRegistry(REGISTRY).agentIdOf(ALICE, sk), agentId);
        }
    }

    /// Set 200 thresholds, then read all of them back. Surfaces any caching
    /// issues in the controller's nested mapping access.
    function test_Stress_SetManyThresholds() public {
        uint256 n = 200;
        for (uint256 i; i < n; ++i) {
            address keyId = address(uint160(0x10000 + i));
            address token = address(uint160(0x20000 + i));
            vm.prank(ALICE);
            AgentBudgetController(BUDGET).setThreshold(keyId, token, i + 1);
        }
        for (uint256 i; i < n; ++i) {
            address keyId = address(uint160(0x10000 + i));
            address token = address(uint160(0x20000 + i));
            assertEq(AgentBudgetController(BUDGET).getThreshold(ALICE, keyId, token), i + 1);
        }
    }

    /// 100 trip/untrip cycles. Verifies the precompile state stays in sync.
    function test_Stress_TripUntripCycles() public {
        uint256 cycles = 100;
        for (uint256 i; i < cycles; ++i) {
            vm.prank(ALICE);
            AgentCircuitBreaker(BREAKER).trip(BOB, bytes32(i));
            assertTrue(AgentCircuitBreaker(BREAKER).isTripped(BOB));

            vm.prank(ALICE);
            AgentCircuitBreaker(BREAKER).untrip(BOB);
            assertFalse(AgentCircuitBreaker(BREAKER).isTripped(BOB));
        }
    }

    /// Authorize, bind, then unbind a batch of keys. Catches transient state
    /// leaks (e.g. if unbindKey didn't fully clear the binding).
    function test_Stress_BindUnbindRoundtrip() public {
        uint256 n = 50;
        address[] memory keys = new address[](n);
        for (uint256 i; i < n; ++i) {
            keys[i] = address(uint160(uint256(keccak256(abi.encode("u-sk", i)))));
            authorizeUnrestricted(ALICE, keys[i]);
            vm.prank(ALICE);
            AgentAccessKeyRegistry(REGISTRY).bindKey(keys[i], keccak256(abi.encode("u-agent", i)));
        }
        for (uint256 i; i < n; ++i) {
            vm.prank(ALICE);
            AgentAccessKeyRegistry(REGISTRY).unbindKey(keys[i]);
            assertFalse(AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, keys[i]));
            assertEq(AgentAccessKeyRegistry(REGISTRY).agentIdOf(ALICE, keys[i]), bytes32(0));
        }
    }

    /// Many guardians granted then revoked. Exercises the _guardians mapping
    /// under heavy churn.
    function test_Stress_GuardianChurn() public {
        uint256 n = 50;
        address[] memory guardians = new address[](n);
        for (uint256 i; i < n; ++i) {
            guardians[i] = address(uint160(0x40000 + i));
            vm.prank(ALICE);
            AgentCircuitBreaker(BREAKER).setGuardian(guardians[i], true);
            assertTrue(AgentCircuitBreaker(BREAKER).isGuardian(guardians[i]));
        }
        for (uint256 i; i < n; ++i) {
            vm.prank(ALICE);
            AgentCircuitBreaker(BREAKER).setGuardian(guardians[i], false);
            assertFalse(AgentCircuitBreaker(BREAKER).isGuardian(guardians[i]));
        }
    }
}
