// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { DevnetBase } from "./base/DevnetBase.sol";
import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";

/// @title  Smoke.t.sol
/// @notice §1 of `sci/docs/p0-2-integration-tests.md` — preflight checks. These
///         are the fastest tests; failure here means the devnet is in a state
///         no other test can cover.
contract SmokeTest is DevnetBase {
    function test_ChainId_IsExpected() public view {
        assertEq(block.chainid, SCI_CHAIN_ID, "must run against SCI chain (--fork-url $L2_RPC)");
    }

    function test_PrecompileMarkers_ArePresent() public view {
        // After `vm.etch` in setUp the addresses now carry our mocks, not the
        // raw `0xef` marker. To inspect the on-chain marker we'd skip the etch
        // — `DeploymentParity.t.sol` does that. Here we only assert non-empty.
        assertGt(KEYCHAIN.code.length, 0);
        assertGt(SCI_AGENT_STATE.code.length, 0);
    }

    function test_Predeploys_HaveSolidityBytecode() public view {
        // All 4 predeploys should have non-trivial bytecode whose first byte is
        // the standard Solidity dispatcher prefix (0x60 PUSH1).
        address[4] memory addrs = [REGISTRY, BUDGET, BREAKER, DELEGATOR];
        for (uint256 i; i < addrs.length; ++i) {
            bytes memory code = addrs[i].code;
            assertGt(code.length, 100, "predeploy bytecode too short");
            assertEq(uint8(code[0]), 0x60, "predeploy missing PUSH1 prefix");
        }
    }

    function test_BreakerOwner_IsAlice() public view {
        // Genesis alloc seeds AgentCircuitBreaker._owner at slot 0 = ALICE.
        // If a different CB_OWNER was used when generating the allocs, this
        // test will surface that.
        address owner = AgentCircuitBreaker(BREAKER).owner();
        assertEq(owner, ALICE, "breaker owner must be alice on default devnet");
    }

    function test_Accounts_AreFunded() public view {
        // ALICE deliberately starts at 0 on a fresh chain (devnet behavior, not
        // a CLAUDE.md test-account-table value). BOB and CHARLIE are funded.
        assertGt(BOB.balance, 1 ether, "bob must have gas for tests");
        assertGt(CHARLIE.balance, 1 ether, "charlie must have gas for tests");
    }
}
