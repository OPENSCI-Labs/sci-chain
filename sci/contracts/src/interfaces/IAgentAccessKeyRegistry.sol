// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  IAgentAccessKeyRegistry
/// @notice Predeploy at 0xBBBB..0001. Binds session keys (`keyId`) to agent identifiers
///         (`agentId`). The keychain precompile owns the access-key state itself; this
///         registry is a thin metadata layer used by the gateway and explorers to map
///         session keys back to logical agents.
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
    error UnauthorizedCaller();
    error ZeroKeyId();
    error ZeroAgentId();

    function bindKey(address keyId, bytes32 agentId) external;

    function unbindKey(address keyId) external;

    function getBinding(address keyId) external view returns (Binding memory);

    function isBound(address keyId) external view returns (bool);

    function agentIdOf(address keyId) external view returns (bytes32);
}
