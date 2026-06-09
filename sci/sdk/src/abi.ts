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
] as const;
