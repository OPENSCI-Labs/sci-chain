/**
 * {@link SciAaClient} — a thin wrapper over a viem transport that prepares, signs, and
 * submits AA (`0x76`) transactions, plus a few keychain/circuit-breaker view helpers.
 *
 * The transaction type is custom, so `eth_estimateGas` does not understand it — the caller
 * must pass `gasLimit` (or rely on the client's `defaultGasLimit`). Everything else
 * (chain id, nonce, EIP-1559 fees) is auto-filled when omitted.
 */
import {
  type Hex,
  type PublicClient,
  type TransactionReceipt,
  type Transport,
  createPublicClient,
} from "viem";
import { privateKeyToAccount } from "viem/accounts";

import type { AaTransaction, AccessListItem, AaCall, SignedAaTransaction } from "./aa.js";
import { signAaTransaction } from "./aa.js";
import {
  accountKeychainAbi,
  agentAccessKeyRegistryAbi,
  agentCircuitBreakerAbi,
} from "./abi.js";
import { type KeyRestrictions, registerAgentKeyCalls } from "./calls.js";
import {
  ACCOUNT_KEYCHAIN_ADDRESS,
  AGENT_ACCESS_KEY_REGISTRY_ADDRESS,
  AGENT_CIRCUIT_BREAKER_ADDRESS,
} from "./constants.js";

/** Config for {@link SciAaClient}. */
export interface SciAaClientConfig {
  /** viem transport pointing at a SCI Chain RPC. */
  transport: Transport;
  /** Session-key private key (the AA tx signer). */
  privateKey: Hex;
  /** Chain id; fetched via `eth_chainId` and cached when omitted. */
  chainId?: number;
  /** Gas limit used when `send`/`prepare` opts omit it (`0x76` can't use `eth_estimateGas`). */
  defaultGasLimit?: bigint;
  /** Priority fee (wei) used when not provided. */
  defaultMaxPriorityFeePerGas?: bigint;
}

/** Options for building/sending an AA transaction. */
export interface SendAaOptions {
  /** The batch of calls to execute atomically. */
  calls: AaCall[];
  /** Root account the calls act on behalf of (agent delegation). Omit to run as the signer. */
  root?: Hex | null;
  /** Fee payer (sponsored gas). Must equal `root`. Omit for the signer to pay gas. */
  feePayer?: Hex | null;
  nonce?: number;
  gasLimit?: bigint;
  maxFeePerGas?: bigint;
  maxPriorityFeePerGas?: bigint;
  accessList?: AccessListItem[];
}

export class SciAaClient {
  /** The session-key account (derived from the private key). */
  readonly address: Hex;

  private readonly client: PublicClient;
  private readonly privateKey: Hex;
  private readonly defaultGasLimit: bigint;
  private readonly defaultMaxPriorityFeePerGas: bigint;
  private chainId?: number;

  constructor(config: SciAaClientConfig) {
    this.address = privateKeyToAccount(config.privateKey).address;
    this.privateKey = config.privateKey;
    this.client = createPublicClient({ transport: config.transport });
    this.chainId = config.chainId;
    this.defaultGasLimit = config.defaultGasLimit ?? 1_000_000n;
    this.defaultMaxPriorityFeePerGas = config.defaultMaxPriorityFeePerGas ?? 1_000_000_000n;
  }

  /** Returns the chain id, fetching + caching it on first use. */
  async getChainId(): Promise<number> {
    if (this.chainId === undefined) this.chainId = await this.client.getChainId();
    return this.chainId;
  }

  /** Fills in chain id, nonce, and EIP-1559 fees for any fields the caller omitted. */
  async prepare(opts: SendAaOptions): Promise<AaTransaction> {
    if (opts.feePayer) {
      if (!opts.root) {
        throw new Error("feePayer requires root to be set (feePayer must equal root)");
      }
      if (opts.feePayer.toLowerCase() !== opts.root.toLowerCase()) {
        throw new Error(
          "feePayer must equal root (sponsored gas is authorized via the keychain on the root account)",
        );
      }
    }

    const chainId = await this.getChainId();
    const nonce =
      opts.nonce ??
      (await this.client.getTransactionCount({ address: this.address, blockTag: "pending" }));
    const maxPriorityFeePerGas = opts.maxPriorityFeePerGas ?? this.defaultMaxPriorityFeePerGas;
    let maxFeePerGas = opts.maxFeePerGas;
    if (maxFeePerGas === undefined) {
      const block = await this.client.getBlock({ blockTag: "latest" });
      const baseFee = block.baseFeePerGas ?? 0n;
      maxFeePerGas = baseFee * 2n + maxPriorityFeePerGas;
    }

    return {
      chainId,
      nonce,
      maxPriorityFeePerGas,
      maxFeePerGas,
      gasLimit: opts.gasLimit ?? this.defaultGasLimit,
      calls: opts.calls,
      accessList: opts.accessList,
      feePayer: opts.feePayer ?? null,
      root: opts.root ?? null,
    };
  }

  /** Prepares + signs (without submitting). */
  async sign(opts: SendAaOptions): Promise<SignedAaTransaction> {
    return signAaTransaction(await this.prepare(opts), this.privateKey);
  }

  /** Prepares, signs, and submits via `eth_sendRawTransaction`; returns the tx hash. */
  async send(opts: SendAaOptions): Promise<Hex> {
    const { raw } = await this.sign(opts);
    return this.client.sendRawTransaction({ serializedTransaction: raw });
  }

  /**
   * Registers a session key under the **Option B** path: authorizes `keyId` on the keychain
   * (and optionally binds `agentId` in the registry) in a single AA tx.
   *
   * The batch is sent as a `root`-unset AA tx, so its calls execute as the signer — therefore
   * **this client must be constructed with the *root* account's private key**, not the session
   * key's. (Registration bootstraps the key, so it cannot be delegated through `root`.)
   */
  async registerKey(params: {
    keyId: Hex;
    restrictions: KeyRestrictions;
    agentId?: Hex | null;
    nonce?: number;
    gasLimit?: bigint;
    maxFeePerGas?: bigint;
    maxPriorityFeePerGas?: bigint;
  }): Promise<Hex> {
    return this.send({
      calls: registerAgentKeyCalls(params),
      nonce: params.nonce,
      gasLimit: params.gasLimit,
      maxFeePerGas: params.maxFeePerGas,
      maxPriorityFeePerGas: params.maxPriorityFeePerGas,
    });
  }

  /** Waits for the transaction receipt. */
  async waitForReceipt(hash: Hex): Promise<TransactionReceipt> {
    return this.client.waitForTransactionReceipt({ hash });
  }

  /** Reads `keys[account][keyId]` from the keychain precompile. */
  async getKey(account: Hex, keyId: Hex) {
    return this.client.readContract({
      address: ACCOUNT_KEYCHAIN_ADDRESS,
      abi: accountKeychainAbi,
      functionName: "getKey",
      args: [account, keyId],
    });
  }

  /** Reads the remaining spending limit for `keys[account][keyId]` on `token`. */
  async getRemainingLimit(account: Hex, keyId: Hex, token: Hex): Promise<bigint> {
    return this.client.readContract({
      address: ACCOUNT_KEYCHAIN_ADDRESS,
      abi: accountKeychainAbi,
      functionName: "getRemainingLimit",
      args: [account, keyId, token],
    });
  }

  /** Reads the remaining spending limit plus the current period's end (unix seconds). */
  async getRemainingLimitWithPeriod(account: Hex, keyId: Hex, token: Hex) {
    return this.client.readContract({
      address: ACCOUNT_KEYCHAIN_ADDRESS,
      abi: accountKeychainAbi,
      functionName: "getRemainingLimitWithPeriod",
      args: [account, keyId, token],
    });
  }

  /** Reads the call scopes for `keys[account][keyId]` (`isScoped` false → unrestricted). */
  async getAllowedCalls(account: Hex, keyId: Hex) {
    return this.client.readContract({
      address: ACCOUNT_KEYCHAIN_ADDRESS,
      abi: accountKeychainAbi,
      functionName: "getAllowedCalls",
      args: [account, keyId],
    });
  }

  /** Reads the registry binding for `keyId` (agentId / account / registeredAt / revoked). */
  async getBinding(keyId: Hex) {
    return this.client.readContract({
      address: AGENT_ACCESS_KEY_REGISTRY_ADDRESS,
      abi: agentAccessKeyRegistryAbi,
      functionName: "getBinding",
      args: [keyId],
    });
  }

  /** Returns whether `keyId` is currently bound to an agent in the registry. */
  async isBound(keyId: Hex): Promise<boolean> {
    return this.client.readContract({
      address: AGENT_ACCESS_KEY_REGISTRY_ADDRESS,
      abi: agentAccessKeyRegistryAbi,
      functionName: "isBound",
      args: [keyId],
    });
  }

  /** Reads the off-chain `agentId` bound to `keyId` (zero if unbound). */
  async agentIdOf(keyId: Hex): Promise<Hex> {
    return this.client.readContract({
      address: AGENT_ACCESS_KEY_REGISTRY_ADDRESS,
      abi: agentAccessKeyRegistryAbi,
      functionName: "agentIdOf",
      args: [keyId],
    });
  }

  /** Returns whether `sessionKey` is currently tripped (frozen) by the circuit breaker. */
  async isTripped(sessionKey: Hex): Promise<boolean> {
    return this.client.readContract({
      address: AGENT_CIRCUIT_BREAKER_ADDRESS,
      abi: agentCircuitBreakerAbi,
      functionName: "isTripped",
      args: [sessionKey],
    });
  }
}
