// SPDX-License-Identifier: MIT
pragma solidity 0.8.15;

import { console2 as console } from "lib/forge-std/src/console2.sol";

import { INitroEnclaveVerifier } from "interfaces/L1/proofs/tee/INitroEnclaveVerifier.sol";
import { IDisputeGameFactory } from "interfaces/L1/proofs/IDisputeGameFactory.sol";
import { IAnchorStateRegistry } from "interfaces/L1/proofs/IAnchorStateRegistry.sol";
import { DevTEEProverRegistry } from "test/mocks/MockDevTEEProverRegistry.sol";
import { GameType } from "src/libraries/bridge/Types.sol";

import { DeployDevBase } from "./DeployDevBase.s.sol";

/// @title DeploySciLive
/// @notice SCI Chain (chainId 42001, L1 = Sepolia) dev-bypass multiproof deployment that
///         attaches to the ALREADY-LIVE OP-Stack infrastructure instead of deploying fresh
///         mocks. It SKIPS `_deployInfrastructure` (no new DisputeGameFactory, no mock
///         AnchorStateRegistry) and instead binds the live, op-deployer-deployed:
///           - DisputeGameFactory  0x69a8e8137d8f5a35ba0670192738816c3031ec52
///           - AnchorStateRegistry 0x38ee07a983f73bc2ad116b6295e46a5ddc675695
///         then deploys the dev TEE registry + TEEVerifier + AggregateVerifier and registers
///         AggregateVerifier as game type 621 on the live factory ("shadow": the proposer can
///         create type-621 games without `respectedGameType` being flipped, so finalization of
///         the existing permissioned game type is unaffected).
///
///         Dev-only (no AWS Nitro attestation): uses DevTEEProverRegistry.addDevSigner.
///         Broadcasting account MUST be the DisputeGameFactory owner
///         (0xd339ffBf98D9f56Fb391f9130986DC5B8a2c282e — verified on-chain 2026-06-24).
///
///         Pre-broadcast runtime check (see deployment plan §3): confirm the live
///         AnchorStateRegistry can serve an anchor for the brand-new game type 621, or that
///         the AggregateVerifier's anchor read tolerates a missing/zero anchor, before the
///         proposer creates its first type-621 game.
contract DeploySciLive is DeployDevBase {
    /// @notice Live OP-Stack DisputeGameFactory proxy on Sepolia (SCI L1).
    address public constant LIVE_DISPUTE_GAME_FACTORY = 0x69A8E8137D8F5a35Ba0670192738816C3031Ec52;
    /// @notice Live OP-Stack AnchorStateRegistry proxy on Sepolia (SCI L1).
    address public constant LIVE_ANCHOR_STATE_REGISTRY = 0x38eE07A983F73BC2ad116b6295E46A5ddC675695;

    uint256 public constant BLOCK_INTERVAL = 600;
    uint256 public constant INTERMEDIATE_BLOCK_INTERVAL = 30;
    uint256 public constant INIT_BOND = 0.001 ether;

    /// @dev Skip deploying fresh infrastructure; bind the live factory + anchor registry.
    function _deployInfrastructure(GameType) internal override {
        disputeGameFactory = LIVE_DISPUTE_GAME_FACTORY;
        mockAnchorRegistry = IAnchorStateRegistry(LIVE_ANCHOR_STATE_REGISTRY);
    }

    function _blockInterval() internal pure override returns (uint256) {
        return BLOCK_INTERVAL;
    }

    function _intermediateBlockInterval() internal pure override returns (uint256) {
        return INTERMEDIATE_BLOCK_INTERVAL;
    }

    function _initBond() internal pure override returns (uint256) {
        return INIT_BOND;
    }

    function _outputSuffix() internal pure override returns (string memory) {
        return "-sci-live.json";
    }

    function _deployTEERegistryImpl() internal override returns (address) {
        return
            address(
                new DevTEEProverRegistry(INitroEnclaveVerifier(address(0)), IDisputeGameFactory(disputeGameFactory))
            );
    }

    function _logHeader() internal view override {
        console.log("=== Deploying SCI Live Multiproof (NO NITRO, shadow on live infra) ===");
        console.log("Chain ID (L1):", block.chainid);
        console.log("L2 Chain ID:", cfg.l2ChainId());
        console.log("Live DisputeGameFactory:", LIVE_DISPUTE_GAME_FACTORY);
        console.log("Live AnchorStateRegistry:", LIVE_ANCHOR_STATE_REGISTRY);
        console.log("Owner / broadcaster:", cfg.finalSystemOwner());
        console.log("TEE Proposer:", cfg.teeProposer());
        console.log("TEE Challenger:", cfg.teeChallenger());
        console.log("Game Type:", cfg.multiproofGameType());
        console.log("Block interval / intermediate:", BLOCK_INTERVAL, INTERMEDIATE_BLOCK_INTERVAL);
    }

    function _printSummary() internal view override {
        console.log("\n=== SCI LIVE DEPLOYMENT COMPLETE (NO NITRO) ===");
        console.log("DevTEEProverRegistry:", teeProverRegistryProxy);
        console.log("TEEVerifier:", teeVerifier);
        console.log("DisputeGameFactory (live):", disputeGameFactory);
        console.log("AnchorStateRegistry (live):", address(mockAnchorRegistry));
        console.log("DelayedWETH (mock):", mockDelayedWETH);
        console.log("AggregateVerifier:", aggregateVerifier);
        console.log("Game Type:", cfg.multiproofGameType());
        console.log("TEE Image Hash:", vm.toString(cfg.teeImageHash()));
        console.log("Config Hash:", vm.toString(cfg.multiproofConfigHash()));
    }
}
