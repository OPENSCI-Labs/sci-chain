// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  IAccountKeychain
/// @notice ABI for the AccountKeychain precompile at 0xAAAA..0000. Selectors are
///         load-bearing: the Rust pre-execution hook decodes calldata against this
///         exact shape. Mirror of `sci/crates/precompile-abi/src/precompiles/account_keychain.rs`.
interface IAccountKeychain {
    enum SignatureType {
        Secp256k1,
        P256,
        WebAuthn
    }

    struct LegacyTokenLimit {
        address token;
        uint256 amount;
    }

    struct TokenLimit {
        address token;
        uint256 amount;
        uint64 period;
    }

    struct SelectorRule {
        bytes4 selector;
        address[] recipients;
    }

    struct CallScope {
        address target;
        SelectorRule[] selectorRules;
    }

    struct KeyRestrictions {
        uint64 expiry;
        bool enforceLimits;
        TokenLimit[] limits;
        bool allowAnyCalls;
        CallScope[] allowedCalls;
    }

    struct KeyInfo {
        SignatureType signatureType;
        address keyId;
        uint64 expiry;
        bool enforceLimits;
        bool isRevoked;
    }

    event KeyAuthorized(
        address indexed account, address indexed publicKey, uint8 signatureType, uint64 expiry
    );
    event KeyRevoked(address indexed account, address indexed publicKey);
    event SpendingLimitUpdated(
        address indexed account, address indexed publicKey, address indexed token, uint256 newLimit
    );
    event AccessKeySpend(
        address indexed account,
        address indexed publicKey,
        address indexed token,
        uint256 amount,
        uint256 remainingLimit
    );
    event KeyAuthorizationWitness(address indexed account, bytes32 indexed witness);
    event KeyAuthorizationWitnessBurned(address indexed account, bytes32 indexed witness);

    error UnauthorizedCaller();
    error KeyAlreadyExists();
    error KeyNotFound();
    error KeyExpired();
    error SpendingLimitExceeded();
    error InvalidSpendingLimit();
    error InvalidSignatureType();
    error ZeroPublicKey();
    error ExpiryInPast();
    error KeyAlreadyRevoked();
    error SignatureTypeMismatch(uint8 expected, uint8 actual);
    error CallNotAllowed();
    error InvalidCallScope();
    error InvalidKeyAuthorizationWitness();
    error KeyAuthorizationWitnessAlreadyBurned();
    error LegacyAuthorizeKeySelectorChanged(bytes4 newSelector);

    /// Legacy authorize-key entrypoint used before T3.
    function authorizeKey(
        address keyId,
        SignatureType signatureType,
        uint64 expiry,
        bool enforceLimits,
        LegacyTokenLimit[] calldata limits
    ) external;

    /// T3 authorize-key with full restrictions.
    function authorizeKey(address keyId, SignatureType signatureType, KeyRestrictions calldata config)
        external;

    /// T5 authorize-key with TIP-1053 witness.
    function authorizeKey(
        address keyId,
        SignatureType signatureType,
        KeyRestrictions calldata config,
        bytes32 witness
    ) external;

    function burnKeyAuthorizationWitness(bytes32 witness) external;

    function revokeKey(address keyId) external;

    function updateSpendingLimit(address keyId, address token, uint256 newLimit) external;

    function setAllowedCalls(address keyId, CallScope[] calldata scopes) external;

    function removeAllowedCalls(address keyId, address target) external;

    function getKey(address account, address keyId) external view returns (KeyInfo memory);

    function getRemainingLimit(address account, address keyId, address token)
        external
        view
        returns (uint256 remaining);

    function getRemainingLimitWithPeriod(address account, address keyId, address token)
        external
        view
        returns (uint256 remaining, uint64 periodEnd);

    function getAllowedCalls(address account, address keyId)
        external
        view
        returns (bool isScoped, CallScope[] memory scopes);

    function isKeyAuthorizationWitnessBurned(address account, bytes32 witness)
        external
        view
        returns (bool);

    function getTransactionKey() external view returns (address);
}
