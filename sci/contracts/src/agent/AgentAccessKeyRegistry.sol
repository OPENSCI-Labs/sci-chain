// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAccountKeychain } from "../interfaces/IAccountKeychain.sol";
import { IAgentAccessKeyRegistry } from "../interfaces/IAgentAccessKeyRegistry.sol";

/// @title  AgentAccessKeyRegistry
/// @notice Predeploy at 0xBBBB..0001. Tracks the binding between a keychain session
///         key (`keyId`) and a logical agent identifier (`agentId`), scoped per root
///         account. The keychain precompile owns the access-key state; this registry
///         is metadata used by the gateway, explorers, and registrars to map session
///         keys back to agents.
///
/// @dev    Trust model: `bindKey`/`unbindKey` operate on `_bindings[msg.sender][keyId]`,
///         so a binding can only ever be created or revoked by the account it is scoped
///         to, and binding requires that account to hold an active keychain key for
///         `keyId`. Keying by `(account, keyId)` (instead of `keyId` globally) removes
///         the squat/front-run vector where an attacker authorizes a victim's session
///         key under the attacker's own root and claims the global binding first.
///
///         A binding is registry metadata only — revoking the keychain key does NOT
///         auto-revoke the binding. Consumers needing live authorization status must
///         check the keychain (`getKey`) in addition to this registry.
contract AgentAccessKeyRegistry is IAgentAccessKeyRegistry {
    address internal constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    mapping(address account => mapping(address keyId => Binding)) private _bindings;

    function bindKey(address keyId, bytes32 agentId) external {
        if (keyId == address(0)) revert ZeroKeyId();
        if (agentId == bytes32(0)) revert ZeroAgentId();

        Binding storage b = _bindings[msg.sender][keyId];
        if (b.agentId != bytes32(0) && !b.revoked) revert AlreadyBound();

        IAccountKeychain.KeyInfo memory info = IAccountKeychain(KEYCHAIN).getKey(msg.sender, keyId);
        // getKey zeroes keyId for missing/revoked keys; expired keys still report their
        // stored expiry, so reject those explicitly.
        if (info.keyId == address(0) || info.isRevoked || info.expiry <= block.timestamp) {
            revert NotBound();
        }

        _bindings[msg.sender][keyId] = Binding({
            agentId: agentId,
            account: msg.sender,
            registeredAt: uint64(block.timestamp),
            revoked: false
        });

        emit KeyBound(msg.sender, keyId, agentId, uint64(block.timestamp));
    }

    function unbindKey(address keyId) external {
        Binding storage b = _bindings[msg.sender][keyId];
        if (b.agentId == bytes32(0) || b.revoked) revert NotBound();

        bytes32 agentId = b.agentId;
        b.revoked = true;

        emit KeyUnbound(msg.sender, keyId, agentId);
    }

    function getBinding(address account, address keyId) external view returns (Binding memory) {
        return _bindings[account][keyId];
    }

    function isBound(address account, address keyId) external view returns (bool) {
        Binding storage b = _bindings[account][keyId];
        return b.agentId != bytes32(0) && !b.revoked;
    }

    function agentIdOf(address account, address keyId) external view returns (bytes32) {
        Binding storage b = _bindings[account][keyId];
        if (b.revoked) return bytes32(0);
        return b.agentId;
    }
}
