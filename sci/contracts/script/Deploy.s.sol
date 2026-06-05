// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Script } from "forge-std/Script.sol";
import { console2 } from "forge-std/console2.sol";

import { AgentAccessKeyRegistry } from "../src/agent/AgentAccessKeyRegistry.sol";
import { AgentBudgetController } from "../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../src/agent/AgentCircuitBreaker.sol";

/// @title  Deploy
/// @notice Broadcast deployment of the SCI agent predeploys to a running L2 RPC.
///
///         This script uses regular `new` deployment, so the 3 fixed-address
///         predeploys (`AgentAccessKeyRegistry` at 0xBBBB..01, `AgentBudgetController`
///         at 0xBBBB..02, `AgentCircuitBreaker` at 0xBBBB..03) land at the deployer's
///         CREATE addresses — NOT at their fixed predeploy addresses. Use this on a
///         vanilla anvil / fresh L2 for smoke testing.
///
///         For deploying TO the fixed predeploy addresses, use
///         `script/export-predeploy-allocs.sh` to produce a genesis-alloc JSON and
///         merge it into the devnet genesis at chain init.
///
/// Usage:
///   forge script script/Deploy.s.sol --rpc-url $L2_RPC --private-key $PRIV --broadcast
contract Deploy is Script {
    /// Deploys the 3 agent predeploys. Use this on chains without genesis-baked predeploys
    /// (e.g. a fresh anvil) when you want them at CREATE addresses.
    function run() external {
        address ownerArg = _resolveOwner();

        vm.startBroadcast();

        AgentAccessKeyRegistry registry = new AgentAccessKeyRegistry();
        AgentBudgetController budget = new AgentBudgetController();
        AgentCircuitBreaker breaker = new AgentCircuitBreaker(ownerArg);

        vm.stopBroadcast();

        console2.log("AgentAccessKeyRegistry    :", address(registry));
        console2.log("AgentBudgetController     :", address(budget));
        console2.log("AgentCircuitBreaker       :", address(breaker));
        console2.log("AgentCircuitBreaker.owner :", ownerArg);
    }

    function _resolveOwner() internal view returns (address) {
        address envOwner = vm.envOr("CB_OWNER", address(0));
        if (envOwner != address(0)) return envOwner;
        // Fall back to the broadcaster address (test-account-0 on devnet by default).
        return msg.sender;
    }
}
