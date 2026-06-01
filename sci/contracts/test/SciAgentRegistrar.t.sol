// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { AgentAccessKeyRegistry } from "../src/agent/AgentAccessKeyRegistry.sol";
import { IAccountKeychain } from "../src/interfaces/IAccountKeychain.sol";
import { SciAgentRegistrar } from "../src/integration/SciAgentRegistrar.sol";
import { MockAccountKeychain } from "./mocks/MockAccountKeychain.sol";

contract SciAgentRegistrarTest is Test {
    address constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    AgentAccessKeyRegistry registry;
    SciAgentRegistrar registrar;
    address rootAccount;
    address sessionKey;
    bytes32 constant AGENT_ID = bytes32("agent-1");

    function setUp() public {
        MockAccountKeychain mock = new MockAccountKeychain();
        vm.etch(KEYCHAIN, address(mock).code);

        registry = new AgentAccessKeyRegistry();
        registrar = new SciAgentRegistrar(address(registry));

        rootAccount = makeAddr("rootAccount");
        sessionKey = makeAddr("sessionKey");
    }

    /// The registrar is meant to be invoked under EIP-7702 delegation, so a direct call
    /// where `msg.sender == registrar` will authorize the key on the *registrar's* account
    /// in the keychain, then attempt `registry.bindKey` from the registrar's address. This
    /// test exercises that direct path end-to-end (`msg.sender == registrar` throughout).
    function test_RegisterEndToEnd() public {
        IAccountKeychain.TokenLimit[] memory limits = new IAccountKeychain.TokenLimit[](0);
        IAccountKeychain.CallScope[] memory scopes = new IAccountKeychain.CallScope[](0);
        IAccountKeychain.KeyRestrictions memory cfg = IAccountKeychain.KeyRestrictions({
            expiry: uint64(block.timestamp + 1 days),
            enforceLimits: false,
            limits: limits,
            allowAnyCalls: true,
            allowedCalls: scopes
        });

        registrar.registerAgent(sessionKey, IAccountKeychain.SignatureType.Secp256k1, cfg, AGENT_ID);

        // Authorized on the keychain at address(registrar) (msg.sender during the inner call).
        IAccountKeychain.KeyInfo memory info =
            IAccountKeychain(KEYCHAIN).getKey(address(registrar), sessionKey);
        assertEq(info.keyId, sessionKey);

        // Bound in the registry at address(registrar) (msg.sender during the bindKey call).
        assertEq(registry.agentIdOf(sessionKey), AGENT_ID);
    }
}
