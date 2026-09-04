/**
 * Encoder for the SCI Chain native account-abstraction transaction type (`0x76`).
 *
 * This is the JS/TS port of the Rust dev tool `sci/tools/aa-txgen` and the on-chain
 * codec in `crates/common/consensus/src/transaction/aa.rs` (`BaseAaTransaction`). It
 * produces byte-identical output so the raw tx is accepted by `eth_sendRawTransaction`.
 *
 * Wire format (EIP-2718 typed transaction), mirroring the EIP-1559 layout but with a
 * `calls[]` batch plus optional `feePayer` / `root` instead of a single `to/value/data`:
 *
 *   signing payload = 0x76 ‖ rlp([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
 *                                  gasLimit, calls, accessList, feePayer, root])
 *   signing hash    = keccak256(signing payload)
 *   signed tx       = 0x76 ‖ rlp([...the 9 fields, yParity, r, s])
 *
 * where `calls` is a list of `[to, value, input]`, `feePayer`/`root` encode as the 20-byte
 * address or, when absent, the empty string (`0x80`), and the empty access list is `0xc0`.
 * Integers (chainId/nonce/fees/gasLimit/value/yParity/r/s) are RLP minimal big-endian.
 */
import { keccak256, toRlp, type Hex } from "viem";
import { sign } from "viem/accounts";

/** SCI AA transaction type id (`0x76`), matching `SCI_AA_TX_TYPE_ID` in `aa.rs`. */
export const SCI_AA_TX_TYPE = 0x76;

/** One inner call inside an AA batch (mirrors `aa::Call`). */
export interface AaCall {
  /** Call target, or `null` for CREATE (encodes as the empty string). */
  to: Hex | null;
  /** Wei value forwarded to the call. */
  value: bigint;
  /** Calldata forwarded to the call (`"0x"` for none). */
  input: Hex;
}

/** One EIP-2930 access-list entry. */
export interface AccessListItem {
  address: Hex;
  storageKeys: Hex[];
}

/** A SCI AA transaction (mirrors `BaseAaTransaction`). */
export interface AaTransaction {
  chainId: number | bigint;
  nonce: number | bigint;
  maxPriorityFeePerGas: bigint;
  maxFeePerGas: bigint;
  gasLimit: number | bigint;
  /** Batch of calls executed atomically. */
  calls: AaCall[];
  /** EIP-2930 access list (defaults to empty). */
  accessList?: AccessListItem[];
  /**
   * Optional fee payer (sponsored gas). When set it must equal `root`; the handler
   * rejects `feePayer != root`. `null`/omitted means the signer pays gas.
   */
  feePayer?: Hex | null;
  /**
   * Optional root account the calls execute on behalf of. The tx is signed by the
   * session key; when `root` is set, the inner calls run with `msg.sender == root`
   * after the keychain authorizes `keys[root][signer]`. `null`/omitted runs the batch
   * as the signer itself.
   */
  root?: Hex | null;
}

/** Output of signing: the raw 2718 tx bytes and the typed-tx hash. */
export interface SignedAaTransaction {
  /** `0x76`-prefixed raw bytes for `eth_sendRawTransaction`. */
  raw: Hex;
  /** Transaction hash = `keccak256(raw)`. */
  hash: Hex;
}

/** A node in the RLP input tree (a leaf hex string or a nested list). */
type RlpTree = Hex | RlpTree[];

/** RLP minimal big-endian encoding of a non-negative integer (`0` → `"0x"` → `0x80`). */
function minimalBytes(value: bigint | number): Hex {
  // Coerce up front so a JS-number `0` (falsy, but !== 0n) doesn't slip past the
  // zero check and emit non-minimal RLP.
  const v = BigInt(value);
  if (v < 0n) throw new Error(`cannot RLP-encode a negative integer: ${v}`);
  if (v === 0n) return "0x";
  let hex = v.toString(16);
  if (hex.length % 2 === 1) hex = `0${hex}`;
  return `0x${hex}`;
}

const ADDRESS_RE = /^0x[0-9a-fA-F]{40}$/;

/**
 * An address field, or the empty string (`0x80`) when absent (`feePayer`/`root`/CREATE).
 *
 * Absence must be expressed as strictly `null`/`undefined`. Any other value must be a
 * well-formed 20-byte address — silently coercing a malformed value (e.g. `""`) to the
 * empty field would change the tx's meaning: an empty `root` executes as the signer
 * (bypassing root delegation) and an empty `to` becomes CREATE with the calldata as
 * initcode.
 */
function addressOrEmpty(addr: Hex | null | undefined): Hex {
  if (addr === null || addr === undefined) return "0x";
  if (!ADDRESS_RE.test(addr)) throw new Error(`invalid address field: ${JSON.stringify(addr)}`);
  return addr.toLowerCase() as Hex;
}

/** A byte string, normalized so `undefined`/`null` become the empty string. */
function bytesOrEmpty(input: Hex | null | undefined): Hex {
  return input && input !== "0x" ? (input.toLowerCase() as Hex) : "0x";
}

/** The 9 RLP fields shared by the signing payload and the signed tx. */
function fieldList(tx: AaTransaction): RlpTree[] {
  const calls: RlpTree[] = tx.calls.map((c) => [
    addressOrEmpty(c.to),
    minimalBytes(c.value),
    bytesOrEmpty(c.input),
  ]);
  const accessList: RlpTree[] = (tx.accessList ?? []).map((it) => [
    it.address.toLowerCase() as Hex,
    it.storageKeys.map((k) => k.toLowerCase() as Hex),
  ]);
  return [
    minimalBytes(BigInt(tx.chainId)),
    minimalBytes(BigInt(tx.nonce)),
    minimalBytes(tx.maxPriorityFeePerGas),
    minimalBytes(tx.maxFeePerGas),
    minimalBytes(BigInt(tx.gasLimit)),
    calls,
    accessList,
    addressOrEmpty(tx.feePayer),
    addressOrEmpty(tx.root),
  ];
}

/** Prefixes the `0x76` type byte to an RLP body. */
function prefixType(rlpBody: Hex): Hex {
  return `0x76${rlpBody.slice(2)}`;
}

/**
 * The unsigned signing payload: `0x76 ‖ rlp([the 9 fields])`. Equivalent to
 * `BaseAaTransaction::encode_for_signing`.
 */
export function encodeUnsignedAaTransaction(tx: AaTransaction): Hex {
  return prefixType(toRlp(fieldList(tx) as never));
}

/**
 * The signing hash `keccak256(0x76 ‖ rlp([the 9 fields]))`. This is what gets signed
 * with the session key's secp256k1 key (equivalent to `tx.signature_hash()`).
 */
export function aaSigningHash(tx: AaTransaction): Hex {
  return keccak256(encodeUnsignedAaTransaction(tx));
}

/**
 * Assembles the signed 2718 tx from a transaction and an already-computed signature.
 * `yParity` is `0` or `1`; `r`/`s` are 32-byte hex (they are RLP minimal-encoded here).
 */
export function encodeSignedAaTransaction(
  tx: AaTransaction,
  signature: { yParity: number; r: Hex; s: Hex },
): SignedAaTransaction {
  const signed: RlpTree[] = [
    ...fieldList(tx),
    minimalBytes(BigInt(signature.yParity)),
    minimalBytes(BigInt(signature.r)),
    minimalBytes(BigInt(signature.s)),
  ];
  const raw = prefixType(toRlp(signed as never));
  return { raw, hash: keccak256(raw) };
}

/**
 * Signs an AA transaction with `privateKey` and returns the raw 2718 bytes + hash.
 *
 * Uses RFC-6979 deterministic, low-S ECDSA (via viem/noble), matching the Rust signer
 * (alloy/k256) byte-for-byte, so the same inputs always yield the same raw tx.
 */
export async function signAaTransaction(
  tx: AaTransaction,
  privateKey: Hex,
): Promise<SignedAaTransaction> {
  const signature = await sign({ hash: aaSigningHash(tx), privateKey });
  const yParity =
    signature.yParity ?? (signature.v !== undefined ? Number(signature.v - 27n) : 0);
  return encodeSignedAaTransaction(tx, { yParity, r: signature.r, s: signature.s });
}
