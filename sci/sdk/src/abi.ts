/**
 * viem-compatible ABI fragments for the contracts/precompiles the SDK talks to.
 *
 * Keychain ABI mirrors `sci/contracts/src/interfaces/IAccountKeychain.sol` — selectors are
 * load-bearing (the Rust pre-execution hook decodes calldata against this exact shape). Only
 * the T3 `authorizeKey(address,uint8,KeyRestrictions)` overload is included to avoid overload
 * ambiguity; it is the one SCI uses.
 */

/** Minimal ERC-20 surface used for transfer/approve calls. */
export const erc20Abi = [
  {
    type: "function",
    name: "transfer",
    stateMutability: "nonpayable",
    inputs: [
      { name: "to", type: "address" },
      { name: "amount", type: "uint256" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    type: "function",
    name: "approve",
    stateMutability: "nonpayable",
    inputs: [
      { name: "spender", type: "address" },
      { name: "amount", type: "uint256" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    type: "function",
    name: "balanceOf",
    stateMutability: "view",
    inputs: [{ name: "account", type: "address" }],
    outputs: [{ name: "", type: "uint256" }],
  },
] as const;

/** AccountKeychain precompile ABI (T3 subset the SDK exposes). */
export const accountKeychainAbi = [
  {
    type: "function",
    name: "authorizeKey",
    stateMutability: "nonpayable",
    inputs: [
      { name: "keyId", type: "address" },
      { name: "signatureType", type: "uint8" },
      {
        name: "config",
        type: "tuple",
        components: [
          { name: "expiry", type: "uint64" },
          { name: "enforceLimits", type: "bool" },
          {
            name: "limits",
            type: "tuple[]",
            components: [
              { name: "token", type: "address" },
              { name: "amount", type: "uint256" },
              { name: "period", type: "uint64" },
            ],
          },
          { name: "allowAnyCalls", type: "bool" },
          {
            name: "allowedCalls",
            type: "tuple[]",
            components: [
              { name: "target", type: "address" },
              {
                name: "selectorRules",
                type: "tuple[]",
                components: [
                  { name: "selector", type: "bytes4" },
                  { name: "recipients", type: "address[]" },
                ],
              },
            ],
          },
        ],
      },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "revokeKey",
    stateMutability: "nonpayable",
    inputs: [{ name: "keyId", type: "address" }],
    outputs: [],
  },
  {
    type: "function",
    name: "updateSpendingLimit",
    stateMutability: "nonpayable",
    inputs: [
      { name: "keyId", type: "address" },
      { name: "token", type: "address" },
      { name: "newLimit", type: "uint256" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "setAllowedCalls",
    stateMutability: "nonpayable",
    inputs: [
      { name: "keyId", type: "address" },
      {
        name: "scopes",
        type: "tuple[]",
        components: [
          { name: "target", type: "address" },
          {
            name: "selectorRules",
            type: "tuple[]",
            components: [
              { name: "selector", type: "bytes4" },
              { name: "recipients", type: "address[]" },
            ],
          },
        ],
      },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "removeAllowedCalls",
    stateMutability: "nonpayable",
    inputs: [
      { name: "keyId", type: "address" },
      { name: "target", type: "address" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "getKey",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
    ],
    outputs: [
      {
        name: "",
        type: "tuple",
        components: [
          { name: "signatureType", type: "uint8" },
          { name: "keyId", type: "address" },
          { name: "expiry", type: "uint64" },
          { name: "enforceLimits", type: "bool" },
          { name: "isRevoked", type: "bool" },
        ],
      },
    ],
  },
  {
    type: "function",
    name: "getRemainingLimit",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
      { name: "token", type: "address" },
    ],
    outputs: [{ name: "", type: "uint256" }],
  },
  {
    type: "function",
    name: "getRemainingLimitWithPeriod",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
      { name: "token", type: "address" },
    ],
    outputs: [
      { name: "remaining", type: "uint256" },
      { name: "periodEnd", type: "uint64" },
    ],
  },
  {
    type: "function",
    name: "getAllowedCalls",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
    ],
    outputs: [
      { name: "isScoped", type: "bool" },
      {
        name: "scopes",
        type: "tuple[]",
        components: [
          { name: "target", type: "address" },
          {
            name: "selectorRules",
            type: "tuple[]",
            components: [
              { name: "selector", type: "bytes4" },
              { name: "recipients", type: "address[]" },
            ],
          },
        ],
      },
    ],
  },
  {
    type: "event",
    name: "KeyAuthorized",
    inputs: [
      { name: "account", type: "address", indexed: true },
      { name: "publicKey", type: "address", indexed: true },
      { name: "signatureType", type: "uint8", indexed: false },
      { name: "expiry", type: "uint64", indexed: false },
    ],
  },
  {
    type: "event",
    name: "KeyRevoked",
    inputs: [
      { name: "account", type: "address", indexed: true },
      { name: "publicKey", type: "address", indexed: true },
    ],
  },
  {
    type: "event",
    name: "SpendingLimitUpdated",
    inputs: [
      { name: "account", type: "address", indexed: true },
      { name: "publicKey", type: "address", indexed: true },
      { name: "token", type: "address", indexed: true },
      { name: "newLimit", type: "uint256", indexed: false },
    ],
  },
  {
    type: "event",
    name: "AccessKeySpend",
    inputs: [
      { name: "account", type: "address", indexed: true },
      { name: "publicKey", type: "address", indexed: true },
      { name: "token", type: "address", indexed: true },
      { name: "amount", type: "uint256", indexed: false },
      { name: "remainingLimit", type: "uint256", indexed: false },
    ],
  },
] as const;

/** AgentCircuitBreaker predeploy ABI (admin facade over SciAgentState). */
export const agentCircuitBreakerAbi = [
  {
    type: "function",
    name: "trip",
    stateMutability: "nonpayable",
    inputs: [
      { name: "sessionKey", type: "address" },
      { name: "reason", type: "bytes32" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "untrip",
    stateMutability: "nonpayable",
    inputs: [{ name: "sessionKey", type: "address" }],
    outputs: [],
  },
  {
    type: "function",
    name: "isTripped",
    stateMutability: "view",
    inputs: [{ name: "sessionKey", type: "address" }],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    type: "event",
    name: "Tripped",
    inputs: [
      { name: "sessionKey", type: "address", indexed: true },
      { name: "by", type: "address", indexed: true },
      { name: "reason", type: "bytes32", indexed: false },
    ],
  },
  {
    type: "event",
    name: "Untripped",
    inputs: [
      { name: "sessionKey", type: "address", indexed: true },
      { name: "by", type: "address", indexed: true },
    ],
  },
] as const;

/**
 * AgentAccessKeyRegistry predeploy ABI (`0xBBBB..0001`).
 *
 * A thin metadata layer mirroring `IAgentAccessKeyRegistry.sol` that binds a session key
 * (`keyId`) to an off-chain agent identifier (`agentId`). The keychain owns the access-key
 * state; this registry is for gateways/explorers to map keys back to logical agents.
 */
export const agentAccessKeyRegistryAbi = [
  {
    type: "function",
    name: "bindKey",
    stateMutability: "nonpayable",
    inputs: [
      { name: "keyId", type: "address" },
      { name: "agentId", type: "bytes32" },
    ],
    outputs: [],
  },
  {
    type: "function",
    name: "unbindKey",
    stateMutability: "nonpayable",
    inputs: [{ name: "keyId", type: "address" }],
    outputs: [],
  },
  {
    type: "function",
    name: "getBinding",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
    ],
    outputs: [
      {
        name: "",
        type: "tuple",
        components: [
          { name: "agentId", type: "bytes32" },
          { name: "account", type: "address" },
          { name: "registeredAt", type: "uint64" },
          { name: "revoked", type: "bool" },
        ],
      },
    ],
  },
  {
    type: "function",
    name: "isBound",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
    ],
    outputs: [{ name: "", type: "bool" }],
  },
  {
    type: "function",
    name: "agentIdOf",
    stateMutability: "view",
    inputs: [
      { name: "account", type: "address" },
      { name: "keyId", type: "address" },
    ],
    outputs: [{ name: "", type: "bytes32" }],
  },
  {
    type: "event",
    name: "KeyBound",
    inputs: [
      { name: "account", type: "address", indexed: true },
      { name: "keyId", type: "address", indexed: true },
      { name: "agentId", type: "bytes32", indexed: true },
      { name: "registeredAt", type: "uint64", indexed: false },
    ],
  },
  {
    type: "event",
    name: "KeyUnbound",
    inputs: [
      { name: "account", type: "address", indexed: true },
      { name: "keyId", type: "address", indexed: true },
      { name: "agentId", type: "bytes32", indexed: true },
    ],
  },
] as const;
