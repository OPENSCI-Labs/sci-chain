// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { DevnetBase } from "./base/DevnetBase.sol";

import { AgentAccessKeyRegistry } from "../../src/agent/AgentAccessKeyRegistry.sol";
import { IAccountKeychain } from "../../src/interfaces/IAccountKeychain.sol";
import { IAgentAccessKeyRegistry } from "../../src/interfaces/IAgentAccessKeyRegistry.sol";

/// @title  RegistryIntegrationTest
/// @notice Tests AgentAccessKeyRegistry binding semantics against a mocked
///         AccountKeychain. The registry is at the genesis-baked address;
///         every test uses fresh `vm.makeAddrAndKey` session keys to stay
///         isolated from sibling tests and any live state on the chain.
contract RegistryIntegrationTest is DevnetBase {
    function _freshKey(string memory seed) internal returns (address k) {
        (k,) = makeAddrAndKey(seed);
    }

    function _seedKeychain(address account, address keyId) internal {
        authorizeUnrestricted(account, keyId);
    }

    function test_BindKey_HappyPath() public {
        address sk = _freshKey("sk-happy");
        bytes32 agentId = id("agent-1");

        _seedKeychain(ALICE, sk);

        vm.prank(ALICE);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, agentId);

        assertTrue(AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, sk));
        assertEq(AgentAccessKeyRegistry(REGISTRY).agentIdOf(ALICE, sk), agentId);

        IAgentAccessKeyRegistry.Binding memory b =
            AgentAccessKeyRegistry(REGISTRY).getBinding(ALICE, sk);
        assertEq(b.agentId, agentId);
        assertEq(b.account, ALICE);
        assertFalse(b.revoked);
    }

    function test_BindKey_RevertsOnZeroKeyId() public {
        vm.prank(ALICE);
        vm.expectRevert(IAgentAccessKeyRegistry.ZeroKeyId.selector);
        AgentAccessKeyRegistry(REGISTRY).bindKey(address(0), id("a"));
    }

    function test_BindKey_RevertsOnZeroAgentId() public {
        address sk = _freshKey("sk-z");
        _seedKeychain(ALICE, sk);

        vm.prank(ALICE);
        vm.expectRevert(IAgentAccessKeyRegistry.ZeroAgentId.selector);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, bytes32(0));
    }

    function test_BindKey_RevertsWhenCallerNotKeychainOwner() public {
        // Alice authorizes the key on HER keychain. Charlie tries to bind.
        // From charlie's perspective the keychain returns empty KeyInfo, so
        // the registry reverts with NotBound.
        address sk = _freshKey("sk-x");
        _seedKeychain(ALICE, sk);

        vm.prank(CHARLIE);
        vm.expectRevert(IAgentAccessKeyRegistry.NotBound.selector);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, id("a"));
    }

    function test_BindKey_AlreadyBound_Reverts() public {
        address sk = _freshKey("sk-dup");
        _seedKeychain(ALICE, sk);

        vm.startPrank(ALICE);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, id("agent-1"));

        vm.expectRevert(IAgentAccessKeyRegistry.AlreadyBound.selector);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, id("agent-2"));
        vm.stopPrank();
    }

    function test_Unbind_ThenRebind_Allowed() public {
        address sk = _freshKey("sk-rebind");
        _seedKeychain(ALICE, sk);

        vm.startPrank(ALICE);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, id("agent-1"));
        AgentAccessKeyRegistry(REGISTRY).unbindKey(sk);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, id("agent-2"));
        vm.stopPrank();

        assertEq(AgentAccessKeyRegistry(REGISTRY).agentIdOf(ALICE, sk), id("agent-2"));
    }

    function test_Unbind_RevertsIfNotBound() public {
        address sk = _freshKey("sk-not-bound");
        vm.prank(ALICE);
        vm.expectRevert(IAgentAccessKeyRegistry.NotBound.selector);
        AgentAccessKeyRegistry(REGISTRY).unbindKey(sk);
    }

    function test_Unbind_RevertsIfCallerNotOriginalBinder() public {
        address sk = _freshKey("sk-other");
        _seedKeychain(ALICE, sk);

        vm.prank(ALICE);
        AgentAccessKeyRegistry(REGISTRY).bindKey(sk, id("agent-1"));

        // Charlie tries to unbind alice's binding. Bindings are keyed per account
        // (M-5), so from charlie's slot there is nothing to unbind.
        vm.prank(CHARLIE);
        vm.expectRevert(IAgentAccessKeyRegistry.NotBound.selector);
        AgentAccessKeyRegistry(REGISTRY).unbindKey(sk);

        assertTrue(AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, sk));
    }

    // -------- Fuzz tests --------

    /// Any non-zero (keyId, agentId) pair where keyId has been authorized on
    /// the keychain by alice should bind cleanly.
    function testFuzz_BindKey_Roundtrip(address keyId, bytes32 agentId) public {
        vm.assume(keyId != address(0));
        vm.assume(agentId != bytes32(0));
        // Avoid colliding with the predeploy addresses (calling authorizeKey on
        // those is meaningless and may interfere with other state).
        vm.assume(keyId != KEYCHAIN && keyId != SCI_AGENT_STATE);
        vm.assume(keyId != REGISTRY && keyId != BUDGET);
        vm.assume(keyId != BREAKER);
        // The fork preserves the registry's live storage; any keyId that's
        // already bound (e.g. from a prior cast walkthrough) would revert with
        // AlreadyBound at the second bindKey call. Skip those.
        vm.assume(!AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, keyId));

        _seedKeychain(ALICE, keyId);

        vm.prank(ALICE);
        AgentAccessKeyRegistry(REGISTRY).bindKey(keyId, agentId);

        assertTrue(AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, keyId));
        assertEq(AgentAccessKeyRegistry(REGISTRY).agentIdOf(ALICE, keyId), agentId);
    }

    /// agentIdOf must equal bytes32(0) after unbind for any previously bound key.
    function testFuzz_Unbind_ClearsAgentIdOf(address keyId, bytes32 agentId) public {
        vm.assume(keyId != address(0));
        vm.assume(agentId != bytes32(0));
        vm.assume(keyId != KEYCHAIN && keyId != SCI_AGENT_STATE);
        vm.assume(keyId != REGISTRY && keyId != BUDGET);
        vm.assume(keyId != BREAKER);
        vm.assume(!AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, keyId));

        _seedKeychain(ALICE, keyId);

        vm.startPrank(ALICE);
        AgentAccessKeyRegistry(REGISTRY).bindKey(keyId, agentId);
        AgentAccessKeyRegistry(REGISTRY).unbindKey(keyId);
        vm.stopPrank();

        assertEq(AgentAccessKeyRegistry(REGISTRY).agentIdOf(ALICE, keyId), bytes32(0));
        assertFalse(AgentAccessKeyRegistry(REGISTRY).isBound(ALICE, keyId));
    }
}
