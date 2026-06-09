/**
 * Golden tests: the JS encoder must produce byte-identical raw txs to the Rust dev tool
 * `sci/tools/aa-txgen` (which uses the real on-chain codec `base-common-consensus`).
 *
 * Vectors were generated with the Anvil test key #0 against chain id 42001:
 *   target/release/sci-aa-txgen <PK> 42001 <nonce> <to> <value>   (+ env overrides)
 *
 * Both signers use RFC-6979 deterministic low-S ECDSA, so signatures (and thus the raw
 * bytes and tx hash) are reproducible across alloy/k256 and viem/noble.
 */
import assert from "node:assert/strict";
import test from "node:test";

import {
  type AaTransaction,
  aaSigningHash,
  encodeUnsignedAaTransaction,
  signAaTransaction,
} from "../src/aa.js";

const PK = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" as const;
const ACC0 = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" as const;
const ACC1 = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8" as const;

interface Vector {
  name: string;
  tx: AaTransaction;
  expectedRaw: `0x${string}`;
  expectedHash: `0x${string}`;
}

const vectors: Vector[] = [
  {
    // sci-aa-txgen <PK> 42001 0 ACC1 1
    name: "simple single call, no fee_payer/root",
    tx: {
      chainId: 42001,
      nonce: 0,
      maxPriorityFeePerGas: 1_000_000n,
      maxFeePerGas: 1_000_000_000n,
      gasLimit: 100_000,
      calls: [{ to: ACC1, value: 1n, input: "0x" }],
    },
    expectedRaw:
      "0x76f87082a41180830f4240843b9aca00830186a0d8d79470997970c51812dc3a010c7d01b50e0d17dc79c80180c0808001a006732b4d9a231a2baed8d3ff174b534574d3c3bd05d0febb08daf6f9bb89a637a0714c0bca7348f949a791fe115c5e64d276d4b102121593bc20c128abf2d38169",
    expectedHash: "0xe75e30a432742a6f8d1e9206aa9889c89761ed172d228dd888dd346dc5bdb93e",
  },
  {
    // MAX_FEE=2e9 MAX_PRIO=1e9 GAS_LIMIT=210000 INPUT=deadbeef FEE_PAYER=ACC0 ROOT=ACC0
    //   sci-aa-txgen <PK> 42001 7 ACC1 1
    name: "fee_payer + root + calldata + custom fees",
    tx: {
      chainId: 42001,
      nonce: 7,
      maxPriorityFeePerGas: 1_000_000_000n,
      maxFeePerGas: 2_000_000_000n,
      gasLimit: 210_000,
      calls: [{ to: ACC1, value: 1n, input: "0xdeadbeef" }],
      feePayer: ACC0,
      root: ACC0,
    },
    expectedRaw:
      "0x76f89d82a41107843b9aca00847735940083033450dcdb9470997970c51812dc3a010c7d01b50e0d17dc79c80184deadbeefc094f39fd6e51aad88f6f4ce6ab8827279cfffb9226694f39fd6e51aad88f6f4ce6ab8827279cfffb9226680a02bf1577aab37986248a72b3b6d9e89cf5a5af78e5f6cab0b2ee6d98e59c1ea70a04eff96cec8ffd6d968a5c102d321f53979bf8502cc57b1467282dbd1c1b8ee87",
    expectedHash: "0xd9125ba8d572768ddaa9ee0bb251359b63efa90c50b4dd174f23e8004c8833a5",
  },
  {
    // CALL2_TO=0x1111...1111   sci-aa-txgen <PK> 42001 3 ACC1 5
    name: "two-call batch",
    tx: {
      chainId: 42001,
      nonce: 3,
      maxPriorityFeePerGas: 1_000_000n,
      maxFeePerGas: 1_000_000_000n,
      gasLimit: 100_000,
      calls: [
        { to: ACC1, value: 5n, input: "0x" },
        { to: "0x1111111111111111111111111111111111111111", value: 0n, input: "0x" },
      ],
    },
    expectedRaw:
      "0x76f88882a41103830f4240843b9aca00830186a0f0d79470997970c51812dc3a010c7d01b50e0d17dc79c80580d79411111111111111111111111111111111111111118080c0808001a0db6381f0db37de09c7290ab38311bb0fe7f29aced6d10a72adfc49b8c4f2eecba01e80d74607d9ad81bc7fd0d652aab2c2227dd866fbfe75987dee1d35dd8782bb",
    expectedHash: "0x8dce9f3283a8e038d6313f5e5734372c9fa6c06123b662cc6b7fb5f69447ea31",
  },
];

for (const v of vectors) {
  test(`signAaTransaction matches Rust golden: ${v.name}`, async () => {
    const { raw, hash } = await signAaTransaction(v.tx, PK);
    assert.equal(raw, v.expectedRaw, "raw 2718 bytes must match sci-aa-txgen");
    assert.equal(hash, v.expectedHash, "tx hash must match sci-aa-txgen");
  });
}

test("encodeUnsignedAaTransaction is the 0x76-prefixed signing payload", () => {
  const tx = vectors[0]!.tx;
  const unsigned = encodeUnsignedAaTransaction(tx);
  assert.ok(unsigned.startsWith("0x76"), "type byte must be 0x76");
  // The signing hash is keccak256 of exactly this payload.
  assert.equal(aaSigningHash(tx).length, 66, "keccak256 hash is 32 bytes");
});
