/** Well-known SCI Chain addresses and identifiers. */
import type { Hex } from "viem";

/** SCI Chain id. */
export const SCI_CHAIN_ID = 42001;

// NOTE: addresses are lowercase. These vanity addresses are not valid EIP-55 checksums,
// and viem's strict address validation (e.g. in `readContract`) rejects mixed-case
// non-checksum strings — lowercase is unambiguous and always accepted.

/** AccountKeychain precompile (Rust). */
export const ACCOUNT_KEYCHAIN_ADDRESS: Hex =
  "0xaaaaaaaa00000000000000000000000000000000";

/** SciAgentState precompile — CircuitBreaker trip state (Rust). */
export const SCI_AGENT_STATE_ADDRESS: Hex =
  "0xaaaaaaaa00000000000000000000000000000001";

/** AgentAccessKeyRegistry predeploy (Solidity). */
export const AGENT_ACCESS_KEY_REGISTRY_ADDRESS: Hex =
  "0xbbbbbbbb00000000000000000000000000000001";

/** AgentBudgetController predeploy (Solidity). */
export const AGENT_BUDGET_CONTROLLER_ADDRESS: Hex =
  "0xbbbbbbbb00000000000000000000000000000002";

/** AgentCircuitBreaker predeploy (Solidity) — admin facade over SciAgentState. */
export const AGENT_CIRCUIT_BREAKER_ADDRESS: Hex =
  "0xbbbbbbbb00000000000000000000000000000003";

/** secp256k1 signature type (the only one SCI session keys use today). */
export const SIGNATURE_TYPE_SECP256K1 = 0;
