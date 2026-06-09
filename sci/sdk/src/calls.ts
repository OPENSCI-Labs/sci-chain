/**
 * Builders that produce {@link AaCall}s for an AA (`0x76`) batch.
 *
 * Each builder returns a `{ to, value, input }` call ready to drop into
 * `AaTransaction.calls`. Token/keychain builders ABI-encode against the shapes the SCI
 * pre-execution hook decodes (`erc20`, `accountKeychain`, `agentCircuitBreaker` in `abi.ts`).
 */
import { type Abi, type Hex, encodeFunctionData } from "viem";

import type { AaCall } from "./aa.js";
import {
  accountKeychainAbi,
  agentCircuitBreakerAbi,
  erc20Abi,
} from "./abi.js";
import {
  ACCOUNT_KEYCHAIN_ADDRESS,
  AGENT_CIRCUIT_BREAKER_ADDRESS,
  SIGNATURE_TYPE_SECP256K1,
} from "./constants.js";

/** A single selector→recipients scope rule (mirrors `IAccountKeychain.SelectorRule`). */
export interface SelectorRule {
  selector: Hex;
  recipients: Hex[];
}

/** A per-target call scope (mirrors `IAccountKeychain.CallScope`). */
export interface CallScope {
  target: Hex;
  selectorRules: SelectorRule[];
}

/** A spending limit (mirrors `IAccountKeychain.TokenLimit`). `address(0)` = native/gas sentinel. */
export interface TokenLimit {
  token: Hex;
  amount: bigint;
  period: bigint;
}

/** Key restrictions for `authorizeKey` (mirrors `IAccountKeychain.KeyRestrictions`). */
export interface KeyRestrictions {
  /** Unix-seconds expiry; use a large value (e.g. 2n ** 64n - 1n) for "no expiry". */
  expiry: bigint;
  enforceLimits: boolean;
  limits: TokenLimit[];
  allowAnyCalls: boolean;
  allowedCalls: CallScope[];
}

/** A native (value-only) transfer call. */
export function nativeTransferCall(to: Hex, value: bigint): AaCall {
  return { to, value, input: "0x" };
}

/** A generic contract call, ABI-encoding `functionName(args)`. */
export function contractCall(params: {
  to: Hex;
  abi: Abi;
  functionName: string;
  args?: readonly unknown[];
  value?: bigint;
}): AaCall {
  const input = encodeFunctionData({
    abi: params.abi,
    functionName: params.functionName,
    args: params.args as never,
  });
  return { to: params.to, value: params.value ?? 0n, input };
}

/** An ERC-20 `transfer(to, amount)` call against `token`. */
export function erc20TransferCall(token: Hex, to: Hex, amount: bigint): AaCall {
  return {
    to: token,
    value: 0n,
    input: encodeFunctionData({ abi: erc20Abi, functionName: "transfer", args: [to, amount] }),
  };
}

/** An ERC-20 `approve(spender, amount)` call against `token`. */
export function erc20ApproveCall(token: Hex, spender: Hex, amount: bigint): AaCall {
  return {
    to: token,
    value: 0n,
    input: encodeFunctionData({ abi: erc20Abi, functionName: "approve", args: [spender, amount] }),
  };
}

/**
 * An `authorizeKey(keyId, secp256k1, restrictions)` call to the keychain precompile.
 *
 * NOTE: this is a keychain *admin* op — it must be sent by the root account itself (a plain
 * tx, or the first call of a `root === signer` AA batch), not delegated via `root`.
 */
export function authorizeKeyCall(keyId: Hex, restrictions: KeyRestrictions): AaCall {
  return {
    to: ACCOUNT_KEYCHAIN_ADDRESS,
    value: 0n,
    input: encodeFunctionData({
      abi: accountKeychainAbi,
      functionName: "authorizeKey",
      args: [keyId, SIGNATURE_TYPE_SECP256K1, restrictions],
    }),
  };
}

/** A `revokeKey(keyId)` call to the keychain precompile (admin op, see {@link authorizeKeyCall}). */
export function revokeKeyCall(keyId: Hex): AaCall {
  return {
    to: ACCOUNT_KEYCHAIN_ADDRESS,
    value: 0n,
    input: encodeFunctionData({ abi: accountKeychainAbi, functionName: "revokeKey", args: [keyId] }),
  };
}

/** An `updateSpendingLimit(keyId, token, newLimit)` call to the keychain precompile (admin op). */
export function updateSpendingLimitCall(keyId: Hex, token: Hex, newLimit: bigint): AaCall {
  return {
    to: ACCOUNT_KEYCHAIN_ADDRESS,
    value: 0n,
    input: encodeFunctionData({
      abi: accountKeychainAbi,
      functionName: "updateSpendingLimit",
      args: [keyId, token, newLimit],
    }),
  };
}

/** A `trip(sessionKey, reason)` call to the CircuitBreaker facade (guardian/owner only). */
export function circuitBreakerTripCall(sessionKey: Hex, reason: Hex): AaCall {
  return {
    to: AGENT_CIRCUIT_BREAKER_ADDRESS,
    value: 0n,
    input: encodeFunctionData({
      abi: agentCircuitBreakerAbi,
      functionName: "trip",
      args: [sessionKey, reason],
    }),
  };
}

/** An `untrip(sessionKey)` call to the CircuitBreaker facade (guardian/owner only). */
export function circuitBreakerUntripCall(sessionKey: Hex): AaCall {
  return {
    to: AGENT_CIRCUIT_BREAKER_ADDRESS,
    value: 0n,
    input: encodeFunctionData({
      abi: agentCircuitBreakerAbi,
      functionName: "untrip",
      args: [sessionKey],
    }),
  };
}
