export {
  SCI_AA_TX_TYPE,
  type AaCall,
  type AccessListItem,
  type AaTransaction,
  type SignedAaTransaction,
  encodeUnsignedAaTransaction,
  aaSigningHash,
  encodeSignedAaTransaction,
  signAaTransaction,
} from "./aa.js";

export {
  SCI_CHAIN_ID,
  ACCOUNT_KEYCHAIN_ADDRESS,
  SCI_AGENT_STATE_ADDRESS,
  AGENT_ACCESS_KEY_REGISTRY_ADDRESS,
  AGENT_BUDGET_CONTROLLER_ADDRESS,
  AGENT_CIRCUIT_BREAKER_ADDRESS,
  SIGNATURE_TYPE_SECP256K1,
} from "./constants.js";

export {
  erc20Abi,
  accountKeychainAbi,
  agentCircuitBreakerAbi,
  agentAccessKeyRegistryAbi,
} from "./abi.js";

export {
  type SelectorRule,
  type CallScope,
  type TokenLimit,
  type KeyRestrictions,
  nativeTransferCall,
  contractCall,
  erc20TransferCall,
  erc20ApproveCall,
  authorizeKeyCall,
  revokeKeyCall,
  updateSpendingLimitCall,
  setAllowedCallsCall,
  removeAllowedCallsCall,
  bindKeyCall,
  unbindKeyCall,
  registerAgentKeyCalls,
  circuitBreakerTripCall,
  circuitBreakerUntripCall,
} from "./calls.js";

export {
  agentEventsAbi,
  type DecodedAgentEvent,
  decodeAgentEvents,
} from "./events.js";

export { SciAaClient, type SciAaClientConfig, type SendAaOptions } from "./client.js";
