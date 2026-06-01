// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Script } from "forge-std/Script.sol";
import { console2 } from "forge-std/console2.sol";

import { AgentAccessKeyRegistry } from "../src/agent/AgentAccessKeyRegistry.sol";
import { AgentBudgetController } from "../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../src/agent/AgentCircuitBreaker.sol";
import { SCIAgentDelegator } from "../src/integration/SCIAgentDelegator.sol";
import { SciAgentRegistrar } from "../src/integration/SciAgentRegistrar.sol";

/// @title  Deploy
/// @notice Broadcast deployment of the 5 SCI base contracts to a running L2 RPC.
///
///         This script uses regular `new` deployment, so the 4 fixed-address
///         predeploys (`AgentAccessKeyRegistry` at 0xBBBB..01, `AgentBudgetController`
///         at 0xBBBB..02, `AgentCircuitBreaker` at 0xBBBB..03, `SCIAgentDelegator` at
///         0xCCCC..01) land at the deployer's CREATE addresses — NOT at their fixed
///         predeploy addresses. Use this script for:
///
///         - Testing against a vanilla anvil / fresh L2 (the Rust pre-execution hook
///           that depends on `SCI_AGENT_DELEGATOR_ADDRESS == 0xCCCC..01` will not work
///           in this case; the hook is a no-op for any delegation pointing elsewhere).
///         - Smoke-deploying `SciAgentRegistrar` on a real SCI devnet where the 4
///           predeploys are already baked in via `sci-allocs.json` — in that case the
///           `--registry` env var should point to the genesis-baked
///           `AgentAccessKeyRegistry` at 0xBBBB..01.
///
///         For deploying TO the fixed predeploy addresses, use
///         `script/export-predeploy-allocs.sh` to produce a genesis-alloc JSON and
///         merge it into the devnet genesis at chain init.
///
/// Usage:
///   forge script script/Deploy.s.sol --rpc-url $L2_RPC --private-key $PRIV --broadcast
///   forge script script/Deploy.s.sol --rpc-url $L2_RPC --private-key $PRIV --broadcast \
///       --sig "runRegistrarOnly(address)" 0xBbBbbBbB00000000000000000000000000000001
contract Deploy is Script {
    /// Deploys all 5 contracts. Use this on chains without genesis-baked predeploys
    /// (e.g. a fresh anvil) when you want all five at CREATE addresses.
    function run() external {
        address ownerArg = _resolveOwner();

        vm.startBroadcast();

        AgentAccessKeyRegistry registry = new AgentAccessKeyRegistry();
        AgentBudgetController budget = new AgentBudgetController();
        AgentCircuitBreaker breaker = new AgentCircuitBreaker(ownerArg);
        SCIAgentDelegator delegator = new SCIAgentDelegator();
        SciAgentRegistrar registrar = new SciAgentRegistrar(address(registry));

        vm.stopBroadcast();

        console2.log("AgentAccessKeyRegistry    :", address(registry));
        console2.log("AgentBudgetController     :", address(budget));
        console2.log("AgentCircuitBreaker       :", address(breaker));
        console2.log("AgentCircuitBreaker.owner :", ownerArg);
        console2.log("SCIAgentDelegator         :", address(delegator));
        console2.log("SciAgentRegistrar         :", address(registrar));
    }

    /// Deploys ONLY the Registrar, taking the existing on-chain registry address as
    /// an argument. Use this on a SCI devnet where the 4 fixed-addr predeploys are
    /// already baked into genesis.
    function runRegistrarOnly(address registryAddress) external {
        vm.startBroadcast();
        SciAgentRegistrar registrar = new SciAgentRegistrar(registryAddress);
        vm.stopBroadcast();

        console2.log("SciAgentRegistrar         :", address(registrar));
        console2.log("  -> registry             :", registryAddress);
    }

    function _resolveOwner() internal view returns (address) {
        address envOwner = vm.envOr("CB_OWNER", address(0));
        if (envOwner != address(0)) return envOwner;
        // Fall back to the broadcaster address (test-account-0 on devnet by default).
        return msg.sender;
    }
}
