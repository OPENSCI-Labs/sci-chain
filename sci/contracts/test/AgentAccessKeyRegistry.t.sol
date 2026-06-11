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

        _seedKey(rootAccount, sessionKey);
    }

    /// Seeds an authorized, far-future-expiry key on the keychain mock.
    function _seedKey(address account, address keyId) internal {
        IAccountKeychain.KeyInfo memory info = IAccountKeychain.KeyInfo({
            signatureType: IAccountKeychain.SignatureType.Secp256k1,
            keyId: keyId,
            expiry: type(uint64).max,
            enforceLimits: false,
            isRevoked: false
        });
        MockAccountKeychain(KEYCHAIN).setKeyInfo(account, keyId, info);
    }

    function test_BindAndQuery() public {
        vm.prank(rootAccount);
        registry.bindKey(sessionKey, AGENT_ID);

        assertTrue(registry.isBound(rootAccount, sessionKey));
        assertEq(registry.agentIdOf(rootAccount, sessionKey), AGENT_ID);

        IAgentAccessKeyRegistry.Binding memory b = registry.getBinding(rootAccount, sessionKey);
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

    function test_RevertWhen_BindExpiredKey() public {
        address expiredKey = makeAddr("expiredKey");
        IAccountKeychain.KeyInfo memory info = IAccountKeychain.KeyInfo({
            signatureType: IAccountKeychain.SignatureType.Secp256k1,
            keyId: expiredKey,
            expiry: uint64(block.timestamp), // expiry <= now → inactive
            enforceLimits: false,
            isRevoked: false
        });
        MockAccountKeychain(KEYCHAIN).setKeyInfo(rootAccount, expiredKey, info);

        vm.prank(rootAccount);
        vm.expectRevert(IAgentAccessKeyRegistry.NotBound.selector);
        registry.bindKey(expiredKey, AGENT_ID);
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

        assertFalse(registry.isBound(rootAccount, sessionKey));
        assertEq(registry.agentIdOf(rootAccount, sessionKey), bytes32(0));
    }

    function test_RevertWhen_UnbindForeignBinding() public {
        vm.prank(rootAccount);
        registry.bindKey(sessionKey, AGENT_ID);

        // A stranger has no binding of their own for this keyId — per-account keying
        // means they cannot touch rootAccount's binding.
        vm.prank(makeAddr("stranger"));
        vm.expectRevert(IAgentAccessKeyRegistry.NotBound.selector);
        registry.unbindKey(sessionKey);

        assertTrue(registry.isBound(rootAccount, sessionKey));
    }

    /// Regression for review finding M-5 (squat/front-run): an attacker who authorizes the
    /// victim's session-key address under their OWN root and binds it first must not block
    /// (or hijack) the victim's binding — bindings are scoped per account.
    function test_SquatterCannotBlockOrHijackVictimBinding() public {
        address attacker = makeAddr("attacker");
        bytes32 attackerAgent = bytes32("evil-agent");

        // Attacker authorizes the victim's session key address under the attacker root
        // (the keychain allows authorizing any non-zero address) and front-runs bindKey.
        _seedKey(attacker, sessionKey);
        vm.prank(attacker);
        registry.bindKey(sessionKey, attackerAgent);

        // The victim's bind still succeeds, scoped to the victim's account…
        vm.prank(rootAccount);
        registry.bindKey(sessionKey, AGENT_ID);

        // …and resolution under the victim's account yields the victim's agent.
        assertEq(registry.agentIdOf(rootAccount, sessionKey), AGENT_ID);
        assertEq(registry.agentIdOf(attacker, sessionKey), attackerAgent);
    }
}
