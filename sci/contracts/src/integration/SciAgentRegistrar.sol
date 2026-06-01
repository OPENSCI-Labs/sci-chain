// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAccountKeychain } from "../interfaces/IAccountKeychain.sol";
import { IAgentAccessKeyRegistry } from "../interfaces/IAgentAccessKeyRegistry.sol";

/// @title  SciAgentRegistrar
/// @notice One-step agent registration helper (ERC-8004 inspired). The root account
///         calls `registerAgent` once to: (1) authorize a new session key on the
///         keychain, and (2) bind that key to an off-chain agent identifier in the
///         `AgentAccessKeyRegistry`. The IDA NFT mint is left as a stub and emitted
///         as an event for follow-up wiring once the IDA contract lands.
///
/// @dev    Not a fixed-address predeploy. Deployed by the genesis script or by a
///         deployer key during devnet bootstrap; the registry address is taken in
///         the constructor and the keychain address is the precompile constant.
contract SciAgentRegistrar {
    address internal constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    IAgentAccessKeyRegistry public immutable registry;

    event AgentRegistered(
        address indexed account,
        address indexed keyId,
        bytes32 indexed agentId,
        uint8 signatureType
    );
    event IDAMintRequested(address indexed account, bytes32 indexed agentId, address indexed keyId);

    constructor(address registryAddress) {
        registry = IAgentAccessKeyRegistry(registryAddress);
    }

    /// One-step agent registration. Must be called by the root account itself — the
    /// keychain treats `msg.sender` as the account being modified, and the registry
    /// also requires `msg.sender == account`.
    function registerAgent(
        address keyId,
        IAccountKeychain.SignatureType signatureType,
        IAccountKeychain.KeyRestrictions calldata config,
        bytes32 agentId
    ) external {
        IAccountKeychain(KEYCHAIN).authorizeKey(keyId, signatureType, config);
        registry.bindKey(keyId, agentId);

        emit AgentRegistered(msg.sender, keyId, agentId, uint8(signatureType));
        emit IDAMintRequested(msg.sender, agentId, keyId);
    }
}
