/**
 * Event-decoding tests. We synthesize raw logs from the SDK's own ABIs (topics via
 * `encodeEventTopics`, data via `encodeAbiParameters`) and assert `decodeAgentEvents`
 * recovers the typed events — and that it drops logs from unrelated contracts.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  type Log,
  encodeAbiParameters,
  encodeEventTopics,
  keccak256,
  toHex,
} from "viem";

import {
  accountKeychainAbi,
  agentAccessKeyRegistryAbi,
  agentCircuitBreakerAbi,
} from "../src/abi.js";
import { decodeAgentEvents } from "../src/events.js";
import {
  ACCOUNT_KEYCHAIN_ADDRESS,
  AGENT_ACCESS_KEY_REGISTRY_ADDRESS,
  AGENT_CIRCUIT_BREAKER_ADDRESS,
} from "../src/constants.js";

const ACC1 = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8" as const;
const KEY = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" as const;
const AGENT_ID = `0x${"ab".repeat(32)}` as const;

/** Minimal Log stub carrying the fields parseEventLogs needs (others cast away). */
function mkLog(address: `0x${string}`, topics: `0x${string}`[], data: `0x${string}`): Log {
  return {
    address,
    topics,
    data,
    blockNumber: 1n,
    blockHash: `0x${"00".repeat(32)}`,
    transactionHash: `0x${"00".repeat(32)}`,
    transactionIndex: 0,
    logIndex: 0,
    removed: false,
  } as unknown as Log;
}

test("decodes a KeyAuthorized event from the keychain", () => {
  const topics = encodeEventTopics({
    abi: accountKeychainAbi,
    eventName: "KeyAuthorized",
    args: { account: ACC1, publicKey: KEY },
  });
  const data = encodeAbiParameters(
    [{ type: "uint8" }, { type: "uint64" }],
    [0, 2n ** 64n - 1n],
  );
  const events = decodeAgentEvents([mkLog(ACCOUNT_KEYCHAIN_ADDRESS, topics, data)]);
  assert.equal(events.length, 1);
  assert.equal(events[0].eventName, "KeyAuthorized");
  const args = events[0].args as { account: string; publicKey: string; expiry: bigint };
  assert.equal(args.account, ACC1);
  assert.equal(args.publicKey, KEY);
  assert.equal(args.expiry, 2n ** 64n - 1n);
});

test("decodes a KeyBound event from the registry", () => {
  const topics = encodeEventTopics({
    abi: agentAccessKeyRegistryAbi,
    eventName: "KeyBound",
    args: { account: ACC1, keyId: KEY, agentId: AGENT_ID },
  });
  const data = encodeAbiParameters([{ type: "uint64" }], [123n]);
  const events = decodeAgentEvents([mkLog(AGENT_ACCESS_KEY_REGISTRY_ADDRESS, topics, data)]);
  assert.equal(events.length, 1);
  assert.equal(events[0].eventName, "KeyBound");
  const args = events[0].args as { agentId: string; registeredAt: bigint };
  assert.equal(args.agentId, AGENT_ID);
  assert.equal(args.registeredAt, 123n);
});

test("decodes a Tripped event from the circuit breaker", () => {
  const reason = `0x${"11".repeat(32)}` as const;
  const topics = encodeEventTopics({
    abi: agentCircuitBreakerAbi,
    eventName: "Tripped",
    args: { sessionKey: KEY, by: ACC1 },
  });
  const data = encodeAbiParameters([{ type: "bytes32" }], [reason]);
  const events = decodeAgentEvents([mkLog(AGENT_CIRCUIT_BREAKER_ADDRESS, topics, data)]);
  assert.equal(events.length, 1);
  assert.equal(events[0].eventName, "Tripped");
});

test("the eventName filter keeps only the requested event", () => {
  const authd = encodeEventTopics({
    abi: accountKeychainAbi,
    eventName: "KeyAuthorized",
    args: { account: ACC1, publicKey: KEY },
  });
  const revoked = encodeEventTopics({
    abi: accountKeychainAbi,
    eventName: "KeyRevoked",
    args: { account: ACC1, publicKey: KEY },
  });
  const logs = [
    mkLog(
      ACCOUNT_KEYCHAIN_ADDRESS,
      authd,
      encodeAbiParameters([{ type: "uint8" }, { type: "uint64" }], [0, 1n]),
    ),
    mkLog(ACCOUNT_KEYCHAIN_ADDRESS, revoked, "0x"),
  ];
  assert.equal(decodeAgentEvents(logs).length, 2);
  const only = decodeAgentEvents(logs, "KeyRevoked");
  assert.equal(only.length, 1);
  assert.equal(only[0].eventName, "KeyRevoked");
});

test("ignores logs from unrelated contracts", () => {
  // A log whose topic0 matches no SCI event (random Transfer-shaped topic).
  const bogus = mkLog(
    "0x9999999999999999999999999999999999999999",
    [keccak256(toHex("SomethingElse(uint256)"))],
    "0x",
  );
  assert.deepEqual(decodeAgentEvents([bogus]), []);
});
