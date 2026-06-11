// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  IAgentAccessKeyRegistry
/// @notice Predeploy at 0xBBBB..0001. Binds session keys (`keyId`) to agent identifiers
///         (`agentId`), scoped per root account. The keychain precompile owns the
///         access-key state itself; this registry is a thin metadata layer used by the
///         gateway and explorers to map session keys back to logical agents.
///
/// @dev    Bindings are keyed by `(account, keyId)` — matching the keychain's own
///         `keys[account][keyId]` model — NOT by `keyId` alone. A global `keyId` key
///         would be squattable: anyone may authorize any address as a session key under
///         their own root, so a first-come-first-served global binding lets an attacker
///         front-run a victim's `bindKey` and hijack the keyId → agentId resolution.
///         Consumers resolving "which agent is this session key?" must know the root
///         account (the gateway always does — it builds the AA tx with `root`), or index
///         the `KeyBound`/`KeyUnbound` events, which carry the account.
interface IAgentAccessKeyRegistry {
    struct Binding {
        bytes32 agentId;
        address account;
        uint64 registeredAt;
        bool revoked;
    }

    event KeyBound(
        address indexed account,
        address indexed keyId,
        bytes32 indexed agentId,
        uint64 registeredAt
    );
    event KeyUnbound(address indexed account, address indexed keyId, bytes32 indexed agentId);

    error AlreadyBound();
    error NotBound();
    error ZeroKeyId();
    error ZeroAgentId();

    /// @notice Binds `keyId` to `agentId` under the caller's account. The caller must
    ///         hold an active (registered, unrevoked, unexpired) keychain key for `keyId`.
    function bindKey(address keyId, bytes32 agentId) external;

    /// @notice Revokes the caller's binding for `keyId`.
    function unbindKey(address keyId) external;

    function getBinding(address account, address keyId) external view returns (Binding memory);

    function isBound(address account, address keyId) external view returns (bool);

    function agentIdOf(address account, address keyId) external view returns (bytes32);
}
