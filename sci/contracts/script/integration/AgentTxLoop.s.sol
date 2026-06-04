// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Script } from "forge-std/Script.sol";
import { Vm } from "forge-std/Vm.sol";
import { console2 } from "forge-std/console2.sol";

import { AgentAccessKeyRegistry } from "../../src/agent/AgentAccessKeyRegistry.sol";
import { AgentBudgetController } from "../../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";
import { IAccountKeychain } from "../../src/interfaces/IAccountKeychain.sol";
import { ISCIAgentDelegator } from "../../src/interfaces/ISCIAgentDelegator.sol";

/// @title  AgentTxLoop.s.sol
/// @notice **Plan B (legacy) flow.** Exercises the agent loop via the EIP-7702
///         path: register → 7702-delegate → `ISCIAgentDelegator.execute(calls)` →
///         trip → execute (rejected) → untrip → execute. Under Plan A the same
///         loop is driven WITHOUT 7702/delegator: `authorizeKey` then a native AA
///         tx (type `0x76`) carrying `calls[]`, submitted via the `sci-aa-txgen`
///         tool + `eth_sendRawTransaction` (a forge script cannot emit the custom
///         AA tx type). The AA-flow end-to-end is the Phase 6 devnet exercise —
///         see the repro in `sci/docs/test/plan-a-status.md` and `sci/devnet/`.
/// @notice Live-broadcast end-to-end exercise of the SCI pre-execution hook
///         (CircuitBreaker → Scope → SpendingLimit). Drives the full
///         register → 7702-delegate → execute → trip → execute (rejected) →
///         untrip → execute sequence against a running SCI devnet.
///
///         This script CANNOT run in forge test or against a forked anvil:
///         the SCI Rust pre-execution hook lives in the EL binary and is not
///         reproducible under EVM-only simulation. Run only against a real
///         SCI chain RPC.
///
/// Usage:
///   export L2_RPC=http://54.255.70.252:7545
///   forge script script/integration/AgentTxLoop.s.sol --rpc-url $L2_RPC --broadcast \
///     -vvv
///
///   Optional env vars (defaults are the devnet test accounts):
///     ALICE_PK  — root account private key (default: hardhat acc0)
///     BOB_PK    — session key private key (default: hardhat acc1)
///
/// Caveats:
///   - EIP-7702 self-auth nonce trap: this script handles it; see CLAUDE.md.
///   - The script mutates devnet state (authorizes a key, binds it, installs
///     7702 delegation on alice, trips/untrips bob). Re-running cleanly
///     requires a teardown step (or accept stale state and use fresh keys).
///   - Inner-call target is AgentBudgetController.setThreshold — the side
///     effect (a written threshold value) is the observable signal that the
///     batch actually ran. Reads back via getThreshold after each phase.
contract AgentTxLoop is Script {
    address constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;
    address constant REGISTRY = 0xbbBbbbBB00000000000000000000000000000001;
    address constant BUDGET = 0xbBbBbBbB00000000000000000000000000000002;
    address constant BREAKER = 0xBbBbbBbB00000000000000000000000000000003;
    address constant DELEGATOR = 0xCcCCCCcC00000000000000000000000000000001;
    address constant NATIVE = address(0);

    uint256 constant DEFAULT_ALICE_PK =
        0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 constant DEFAULT_BOB_PK =
        0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;

    function run() external {
        uint256 alicePk = vm.envOr("ALICE_PK", DEFAULT_ALICE_PK);
        uint256 bobPk = vm.envOr("BOB_PK", DEFAULT_BOB_PK);
        address alice = vm.addr(alicePk);
        address bob = vm.addr(bobPk);

        console2.log("== AgentTxLoop ==");
        console2.log("  alice :", alice);
        console2.log("  bob   :", bob);

        // ----- Phase 1: fund alice if she has no gas -----
        if (alice.balance < 0.1 ether) {
            console2.log("[1] funding alice from bob");
            vm.broadcast(bobPk);
            (bool ok,) = alice.call{ value: 1 ether }("");
            require(ok, "fund alice failed");
        } else {
            console2.log("[1] alice already funded:", alice.balance);
        }

        // ----- Phase 2: alice authorizes bob -----
        IAccountKeychain.TokenLimit[] memory noLimits = new IAccountKeychain.TokenLimit[](0);
        IAccountKeychain.CallScope[] memory noScopes = new IAccountKeychain.CallScope[](0);
        IAccountKeychain.KeyRestrictions memory cfg = IAccountKeychain.KeyRestrictions({
            expiry: uint64(block.timestamp + 1 days),
            enforceLimits: false,
            limits: noLimits,
            allowAnyCalls: true,
            allowedCalls: noScopes
        });

        console2.log("[2] alice authorizes bob on keychain");
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).authorizeKey(bob, IAccountKeychain.SignatureType.Secp256k1, cfg);

        // ----- Phase 3: alice binds bob to agentId -----
        console2.log("[3] alice binds bob -> agent-1 in registry");
        vm.broadcast(alicePk);
        AgentAccessKeyRegistry(REGISTRY).bindKey(bob, bytes32("agent-1"));

        // ----- Phase 4: alice installs EIP-7702 delegation to delegator -----
        // CRITICAL: for self-auth, the auth's nonce must equal the post-increment
        // tx nonce — i.e. current_nonce + 1. See CLAUDE.md "EIP-7702 self-auth
        // nonce trap (cast)". forge-std exposes signDelegation(implementation, pk)
        // which signs with the correct post-increment nonce automatically when
        // followed by attachDelegation.
        console2.log("[4] alice installs 7702 delegation to delegator");
        Vm.SignedDelegation memory signed = vm.signDelegation(DELEGATOR, alicePk);
        vm.broadcast(alicePk);
        vm.attachDelegation(signed);
        // Trigger the auth via a 0-value self-call. attachDelegation queues
        // the delegation onto the next broadcast tx from alicePk.
        vm.broadcast(alicePk);
        (bool d,) = alice.call{ value: 0 }("");
        require(d, "7702 install tx failed");

        // ----- Phase 5: happy-path execute -----
        bytes memory inner1 = abi.encodeWithSelector(
            AgentBudgetController.setThreshold.selector, bob, NATIVE, 12345
        );
        ISCIAgentDelegator.Call[] memory batch1 = new ISCIAgentDelegator.Call[](1);
        batch1[0] = ISCIAgentDelegator.Call({ target: BUDGET, value: 0, data: inner1 });

        console2.log("[5] bob executes batch via alice (threshold -> 12345)");
        vm.broadcast(bobPk);
        ISCIAgentDelegator(alice).execute(batch1);

        uint256 t1 = AgentBudgetController(BUDGET).getThreshold(alice, bob, NATIVE);
        console2.log("    -> threshold after phase 5:", t1);
        require(t1 == 12345, "happy-path execute did not apply threshold");

        // ----- Phase 6: trip bob -----
        console2.log("[6] alice trips bob");
        vm.broadcast(alicePk);
        AgentCircuitBreaker(BREAKER).trip(bob, bytes32("manual-trip"));
        require(AgentCircuitBreaker(BREAKER).isTripped(bob), "trip did not stick");

        // ----- Phase 7: execute should now be rejected by the SCI hook -----
        // We DO NOT broadcast this tx — the Rust hook rejects at estimateGas,
        // so the broadcast would never land and would surface as a script
        // failure. Instead we log the intended action and let the operator
        // verify out-of-band via cast.
        //
        // NOTE: forge script -vvv shows a simulation trace; the simulation
        // does NOT include the SCI hook (because forge uses anvil locally),
        // so the simulation will "succeed" and broadcast will fail with
        // status=0 or estimate revert. To validate this phase, run the
        // identical command from `sci/devnet/E2E.md` §4 instead.
        console2.log("[7] expected hook rejection -- verify out-of-band with cast");
        console2.log("    threshold remains:", t1);

        // ----- Phase 8: untrip bob -----
        console2.log("[8] alice untrips bob");
        vm.broadcast(alicePk);
        AgentCircuitBreaker(BREAKER).untrip(bob);
        require(!AgentCircuitBreaker(BREAKER).isTripped(bob), "untrip did not stick");

        // ----- Phase 9: execute again, expect threshold flips to 99999 -----
        bytes memory inner2 = abi.encodeWithSelector(
            AgentBudgetController.setThreshold.selector, bob, NATIVE, 99_999
        );
        ISCIAgentDelegator.Call[] memory batch2 = new ISCIAgentDelegator.Call[](1);
        batch2[0] = ISCIAgentDelegator.Call({ target: BUDGET, value: 0, data: inner2 });

        console2.log("[9] bob re-executes after untrip (threshold -> 99999)");
        vm.broadcast(bobPk);
        ISCIAgentDelegator(alice).execute(batch2);

        uint256 t3 = AgentBudgetController(BUDGET).getThreshold(alice, bob, NATIVE);
        console2.log("    -> threshold after phase 9:", t3);
        require(t3 == 99_999, "post-untrip execute did not apply threshold");

        console2.log("== AgentTxLoop complete ==");
    }
}
