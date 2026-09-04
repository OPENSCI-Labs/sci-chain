/**
 * Decode the keychain / registry / circuit-breaker events emitted by an AA tx.
 *
 * The Rust precompile and the Solidity facades emit standard EVM logs, so a receipt's
 * `logs` can be decoded with the SDK's ABIs. {@link decodeAgentEvents} runs viem's
 * `parseEventLogs` over the union of {@link accountKeychainAbi}, {@link agentAccessKeyRegistryAbi},
 * and {@link agentCircuitBreakerAbi}, returning only the recognized, typed events (logs from
 * unrelated contracts are dropped).
 */
import { type Log, parseEventLogs } from "viem";

import {
  accountKeychainAbi,
  agentAccessKeyRegistryAbi,
  agentCircuitBreakerAbi,
} from "./abi.js";

/** The union ABI of every event the SDK knows how to decode. */
export const agentEventsAbi = [
  ...accountKeychainAbi,
  ...agentAccessKeyRegistryAbi,
  ...agentCircuitBreakerAbi,
] as const;

/** Decoded SCI agent events, in `logs` order. */
export type DecodedAgentEvent = ReturnType<
  typeof parseEventLogs<typeof agentEventsAbi, true, undefined>
>[number];

/**
 * Decodes the recognized keychain/registry/circuit-breaker events out of a receipt's logs.
 *
 * @param logs - `receipt.logs` (or any array of raw EVM logs).
 * @param eventName - optional filter; pass an event name to keep only that event.
 */
export function decodeAgentEvents(
  logs: Log[],
  eventName?: DecodedAgentEvent["eventName"],
): DecodedAgentEvent[] {
  return parseEventLogs({
    abi: agentEventsAbi,
    logs,
    ...(eventName ? { eventName } : {}),
  }) as DecodedAgentEvent[];
}
