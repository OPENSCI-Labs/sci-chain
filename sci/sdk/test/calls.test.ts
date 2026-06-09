/**
 * Call-builder tests. Selectors are golden values cross-checked against the canonical
 * Solidity signatures in `IAccountKeychain.sol` / `IAgentCircuitBreaker.sol` (the chain's
 * dispatch decodes calldata against these exact shapes), so an encoding drift fails here.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { decodeFunctionData } from "viem";

import { accountKeychainAbi, erc20Abi } from "../src/abi.js";
import {
  type KeyRestrictions,
  authorizeKeyCall,
  circuitBreakerTripCall,
  circuitBreakerUntripCall,
  erc20ApproveCall,
  erc20TransferCall,
  nativeTransferCall,
  revokeKeyCall,
  updateSpendingLimitCall,
} from "../src/calls.js";
import {
  ACCOUNT_KEYCHAIN_ADDRESS,
  AGENT_CIRCUIT_BREAKER_ADDRESS,
} from "../src/constants.js";

const TOKEN = "0x1111111111111111111111111111111111111111" as const;
const TO = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8" as const;
const KEY = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" as const;

const selectorOf = (input: `0x${string}`) => input.slice(0, 10);

test("nativeTransferCall", () => {
  const call = nativeTransferCall(TO, 123n);
  assert.deepEqual(call, { to: TO, value: 123n, input: "0x" });
});

test("erc20TransferCall encodes transfer(address,uint256)", () => {
  const call = erc20TransferCall(TOKEN, TO, 50n);
  assert.equal(call.to, TOKEN);
  assert.equal(call.value, 0n);
  assert.equal(selectorOf(call.input), "0xa9059cbb");
  const { functionName, args } = decodeFunctionData({ abi: erc20Abi, data: call.input });
  assert.equal(functionName, "transfer");
  assert.deepEqual(args, [TO, 50n]);
});

test("erc20ApproveCall encodes approve(address,uint256)", () => {
  const call = erc20ApproveCall(TOKEN, TO, 7n);
  assert.equal(selectorOf(call.input), "0x095ea7b3");
});

test("authorizeKeyCall (T3) selector + round-trip", () => {
  const restrictions: KeyRestrictions = {
    expiry: 2n ** 64n - 1n,
    enforceLimits: false,
    limits: [],
    allowAnyCalls: true,
    allowedCalls: [],
  };
  const call = authorizeKeyCall(KEY, restrictions);
  assert.equal(call.to, ACCOUNT_KEYCHAIN_ADDRESS);
  assert.equal(selectorOf(call.input), "0x980a6025");
  const { functionName, args } = decodeFunctionData({ abi: accountKeychainAbi, data: call.input });
  assert.equal(functionName, "authorizeKey");
  assert.equal(args[0], KEY);
  assert.equal(args[1], 0); // secp256k1
  assert.equal(args[2].expiry, restrictions.expiry);
  assert.equal(args[2].allowAnyCalls, true);
});

test("revokeKeyCall selector", () => {
  const call = revokeKeyCall(KEY);
  assert.equal(call.to, ACCOUNT_KEYCHAIN_ADDRESS);
  assert.equal(selectorOf(call.input), "0x5ae7ab32");
});

test("updateSpendingLimitCall selector", () => {
  const call = updateSpendingLimitCall(KEY, TOKEN, 1000n);
  assert.equal(selectorOf(call.input), "0xcbbb4480");
});

test("circuitBreaker trip/untrip selectors + target", () => {
  const trip = circuitBreakerTripCall(KEY, `0x${"00".repeat(32)}`);
  assert.equal(trip.to, AGENT_CIRCUIT_BREAKER_ADDRESS);
  assert.equal(selectorOf(trip.input), "0xb07c37ec");

  const untrip = circuitBreakerUntripCall(KEY);
  assert.equal(untrip.to, AGENT_CIRCUIT_BREAKER_ADDRESS);
  assert.equal(selectorOf(untrip.input), "0x81836e78");
});
