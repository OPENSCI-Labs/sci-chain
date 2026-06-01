// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAccountKeychain } from "../../src/interfaces/IAccountKeychain.sol";

/// @notice Forge-side stand-in for the Rust AccountKeychain precompile. Tests etch
///         this contract's runtime bytecode at 0xaAAAaaAA..0000 via `vm.etch`.
contract MockAccountKeychain is IAccountKeychain {
    address private _transactionKey;
    mapping(address => mapping(address => KeyInfo)) private _keys;
    mapping(address => mapping(address => mapping(address => uint256))) private _remaining;
    mapping(address => mapping(address => mapping(address => uint64))) private _periodEnd;

    // For setAllowedCalls / getAllowedCalls: a flat scope-set per (account, keyId,
    // target). Mock keeps a flat list of targets per (account, keyId) so we can
    // round-trip getAllowedCalls. Real precompile uses richer storage.
    mapping(address => mapping(address => bool)) private _isScoped;
    mapping(address => mapping(address => address[])) private _scopedTargets;
    mapping(address => mapping(address => mapping(address => SelectorRule[]))) private _selectorRules;

    // T5 witness state.
    mapping(address => mapping(bytes32 => bool)) private _burnedWitness;

    function setTransactionKey(address k) external {
        _transactionKey = k;
    }

    function getTransactionKey() external view returns (address) {
        return _transactionKey;
    }

    function setKeyInfo(address account, address keyId, KeyInfo memory info) external {
        _keys[account][keyId] = info;
    }

    function getKey(address account, address keyId) external view returns (KeyInfo memory) {
        return _keys[account][keyId];
    }

    function setRemainingLimit(
        address account,
        address keyId,
        address token,
        uint256 amount,
        uint64 periodEnd_
    ) external {
        _remaining[account][keyId][token] = amount;
        _periodEnd[account][keyId][token] = periodEnd_;
    }

    function getRemainingLimit(address account, address keyId, address token)
        external
        view
        returns (uint256)
    {
        return _remaining[account][keyId][token];
    }

    function getRemainingLimitWithPeriod(address account, address keyId, address token)
        external
        view
        returns (uint256 remaining, uint64 periodEnd)
    {
        return (_remaining[account][keyId][token], _periodEnd[account][keyId][token]);
    }

    // --- unused IAccountKeychain methods: revert to make sure tests don't rely on them ---

    function authorizeKey(address, SignatureType, uint64, bool, LegacyTokenLimit[] calldata)
        external
        pure
    {
        revert("mock: legacy authorizeKey not implemented");
    }

    function authorizeKey(address keyId, SignatureType signatureType, KeyRestrictions calldata config)
        external
    {
        _authorize(msg.sender, keyId, signatureType, config);
    }

    function authorizeKey(
        address keyId,
        SignatureType signatureType,
        KeyRestrictions calldata config,
        bytes32 witness
    ) external {
        _authorize(msg.sender, keyId, signatureType, config);
        if (witness != bytes32(0)) {
            emit KeyAuthorizationWitness(msg.sender, witness);
        }
    }

    function _authorize(
        address account,
        address keyId,
        SignatureType signatureType,
        KeyRestrictions calldata config
    ) internal {
        _keys[account][keyId] = KeyInfo({
            signatureType: signatureType,
            keyId: keyId,
            expiry: config.expiry,
            enforceLimits: config.enforceLimits,
            isRevoked: false
        });
        emit KeyAuthorized(account, keyId, uint8(signatureType), config.expiry);

        // Seed per-token spending limits.
        for (uint256 i; i < config.limits.length; ++i) {
            _remaining[account][keyId][config.limits[i].token] = config.limits[i].amount;
            _periodEnd[account][keyId][config.limits[i].token] = 0;
            emit SpendingLimitUpdated(
                account, keyId, config.limits[i].token, config.limits[i].amount
            );
        }

        // Seed call scopes (allowAnyCalls = false ⇒ store the scope set).
        if (!config.allowAnyCalls) {
            _isScoped[account][keyId] = true;
            // Reset any prior scope state for this key.
            address[] storage targets = _scopedTargets[account][keyId];
            for (uint256 i; i < targets.length; ++i) {
                delete _selectorRules[account][keyId][targets[i]];
            }
            delete _scopedTargets[account][keyId];

            for (uint256 i; i < config.allowedCalls.length; ++i) {
                _scopedTargets[account][keyId].push(config.allowedCalls[i].target);
                SelectorRule[] storage dst =
                    _selectorRules[account][keyId][config.allowedCalls[i].target];
                for (uint256 j; j < config.allowedCalls[i].selectorRules.length; ++j) {
                    dst.push(config.allowedCalls[i].selectorRules[j]);
                }
            }
        } else {
            _isScoped[account][keyId] = false;
        }
    }

    function burnKeyAuthorizationWitness(bytes32 witness) external {
        _burnedWitness[msg.sender][witness] = true;
        emit KeyAuthorizationWitnessBurned(msg.sender, witness);
    }

    function revokeKey(address keyId) external {
        _keys[msg.sender][keyId].isRevoked = true;
        emit KeyRevoked(msg.sender, keyId);
    }

    function updateSpendingLimit(address keyId, address token, uint256 newLimit) external {
        _remaining[msg.sender][keyId][token] = newLimit;
        emit SpendingLimitUpdated(msg.sender, keyId, token, newLimit);
    }

    function setAllowedCalls(address keyId, CallScope[] calldata scopes) external {
        _isScoped[msg.sender][keyId] = true;
        // Clear prior state.
        address[] storage targets = _scopedTargets[msg.sender][keyId];
        for (uint256 i; i < targets.length; ++i) {
            delete _selectorRules[msg.sender][keyId][targets[i]];
        }
        delete _scopedTargets[msg.sender][keyId];

        for (uint256 i; i < scopes.length; ++i) {
            _scopedTargets[msg.sender][keyId].push(scopes[i].target);
            SelectorRule[] storage dst = _selectorRules[msg.sender][keyId][scopes[i].target];
            for (uint256 j; j < scopes[i].selectorRules.length; ++j) {
                dst.push(scopes[i].selectorRules[j]);
            }
        }
    }

    function removeAllowedCalls(address keyId, address target) external {
        delete _selectorRules[msg.sender][keyId][target];
        // Remove from the targets list.
        address[] storage targets = _scopedTargets[msg.sender][keyId];
        for (uint256 i; i < targets.length; ++i) {
            if (targets[i] == target) {
                targets[i] = targets[targets.length - 1];
                targets.pop();
                break;
            }
        }
    }

    function getAllowedCalls(address account, address keyId)
        external
        view
        returns (bool, CallScope[] memory)
    {
        bool scoped = _isScoped[account][keyId];
        address[] storage targets = _scopedTargets[account][keyId];
        CallScope[] memory out = new CallScope[](targets.length);
        for (uint256 i; i < targets.length; ++i) {
            out[i] = CallScope({
                target: targets[i],
                selectorRules: _selectorRules[account][keyId][targets[i]]
            });
        }
        return (scoped, out);
    }

    function isKeyAuthorizationWitnessBurned(address account, bytes32 witness)
        external
        view
        returns (bool)
    {
        return _burnedWitness[account][witness];
    }
}
