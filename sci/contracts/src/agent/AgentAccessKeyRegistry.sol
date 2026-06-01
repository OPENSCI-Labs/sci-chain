// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAccountKeychain } from "../interfaces/IAccountKeychain.sol";
import { IAgentAccessKeyRegistry } from "../interfaces/IAgentAccessKeyRegistry.sol";

/// @title  AgentAccessKeyRegistry
/// @notice Predeploy at 0xBBBB..0001. Tracks the binding between a keychain session
///         key (`keyId`) and a logical agent identifier (`agentId`). The keychain
///         precompile owns the access-key state; this registry is metadata used by
///         the gateway, explorers, and registrars to map session keys back to agents.
///
/// @dev    Trust model: the caller must either be the keychain account that owns
///         `keyId` (i.e. the root account that ran `authorizeKey`) or a registrar
///         contract acting on its behalf. The latter is detected by `tx.origin ==
///         keychain owner of keyId` — but rather than introspect the keychain we keep
///         it simple and require `account == msg.sender`. Registrars must therefore
///         be invoked from the root account directly (which matches the EIP-7702 flow
///         where the root tx is signed by the root key).
contract AgentAccessKeyRegistry is IAgentAccessKeyRegistry {
    address internal constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    mapping(address => Binding) private _bindings;

    function bindKey(address keyId, bytes32 agentId) external {
        if (keyId == address(0)) revert ZeroKeyId();
        if (agentId == bytes32(0)) revert ZeroAgentId();

        Binding storage b = _bindings[keyId];
        if (b.agentId != bytes32(0) && !b.revoked) revert AlreadyBound();

        IAccountKeychain.KeyInfo memory info = IAccountKeychain(KEYCHAIN).getKey(msg.sender, keyId);
        if (info.keyId == address(0) || info.isRevoked) revert NotBound();

        _bindings[keyId] = Binding({
            agentId: agentId,
            account: msg.sender,
            registeredAt: uint64(block.timestamp),
            revoked: false
        });

        emit KeyBound(msg.sender, keyId, agentId, uint64(block.timestamp));
    }

    function unbindKey(address keyId) external {
        Binding storage b = _bindings[keyId];
        if (b.agentId == bytes32(0) || b.revoked) revert NotBound();
        if (b.account != msg.sender) revert UnauthorizedCaller();

        bytes32 agentId = b.agentId;
        b.revoked = true;

        emit KeyUnbound(msg.sender, keyId, agentId);
    }

    function getBinding(address keyId) external view returns (Binding memory) {
        return _bindings[keyId];
    }

    function isBound(address keyId) external view returns (bool) {
        Binding storage b = _bindings[keyId];
        return b.agentId != bytes32(0) && !b.revoked;
    }

    function agentIdOf(address keyId) external view returns (bytes32) {
        Binding storage b = _bindings[keyId];
        if (b.revoked) return bytes32(0);
        return b.agentId;
    }
}
