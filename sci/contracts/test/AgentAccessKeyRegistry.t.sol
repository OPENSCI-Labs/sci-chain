// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { AgentAccessKeyRegistry } from "../src/agent/AgentAccessKeyRegistry.sol";
import { IAccountKeychain } from "../src/interfaces/IAccountKeychain.sol";
import { IAgentAccessKeyRegistry } from "../src/interfaces/IAgentAccessKeyRegistry.sol";
import { MockAccountKeychain } from "./mocks/MockAccountKeychain.sol";

contract AgentAccessKeyRegistryTest is Test {
    address constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    AgentAccessKeyRegistry registry;
    address rootAccount;
    address sessionKey;
    bytes32 constant AGENT_ID = bytes32("agent-1");

    function setUp() public {
        MockAccountKeychain mock = new MockAccountKeychain();
        vm.etch(KEYCHAIN, address(mock).code);

        registry = new AgentAccessKeyRegistry();
        rootAccount = makeAddr("rootAccount");
        sessionKey = makeAddr("sessionKey");

        // Seed an authorized key on the keychain mock at the root account.
        IAccountKeychain.KeyInfo memory info = IAccountKeychain.KeyInfo({
            signatureType: IAccountKeychain.SignatureType.Secp256k1,
            keyId: sessionKey,
            expiry: type(uint64).max,
            enforceLimits: false,
            isRevoked: false
        });
        MockAccountKeychain(KEYCHAIN).setKeyInfo(rootAccount, sessionKey, info);
    }

    function test_BindAndQuery() public {
        vm.prank(rootAccount);
        registry.bindKey(sessionKey, AGENT_ID);

        assertTrue(registry.isBound(sessionKey));
        assertEq(registry.agentIdOf(sessionKey), AGENT_ID);

        IAgentAccessKeyRegistry.Binding memory b = registry.getBinding(sessionKey);
        assertEq(b.agentId, AGENT_ID);
        assertEq(b.account, rootAccount);
        assertFalse(b.revoked);
    }

    function test_RevertWhen_BindZeroKey() public {
        vm.prank(rootAccount);
        vm.expectRevert(IAgentAccessKeyRegistry.ZeroKeyId.selector);
        registry.bindKey(address(0), AGENT_ID);
    }

    function test_RevertWhen_BindZeroAgentId() public {
        vm.prank(rootAccount);
        vm.expectRevert(IAgentAccessKeyRegistry.ZeroAgentId.selector);
        registry.bindKey(sessionKey, bytes32(0));
    }

    function test_RevertWhen_BindUnauthorizedKey() public {
        // Caller is not the keychain owner of `sessionKey` — keychain returns empty KeyInfo.
        vm.prank(makeAddr("stranger"));
        vm.expectRevert(IAgentAccessKeyRegistry.NotBound.selector);
        registry.bindKey(sessionKey, AGENT_ID);
    }

    function test_RebindRevertsWhenAlreadyBound() public {
        vm.prank(rootAccount);
        registry.bindKey(sessionKey, AGENT_ID);

        vm.prank(rootAccount);
        vm.expectRevert(IAgentAccessKeyRegistry.AlreadyBound.selector);
        registry.bindKey(sessionKey, bytes32("agent-2"));
    }

    function test_Unbind() public {
        vm.prank(rootAccount);
        registry.bindKey(sessionKey, AGENT_ID);

        vm.prank(rootAccount);
        registry.unbindKey(sessionKey);

        assertFalse(registry.isBound(sessionKey));
        assertEq(registry.agentIdOf(sessionKey), bytes32(0));
    }
}
