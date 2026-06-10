/**
 * SciAaClient tests using a viem mock transport (no live node). They verify the
 * prepare → sign → submit path produces the encoder's exact bytes, that nonce is
 * auto-filled, and that the feePayer/root invariant is enforced client-side.
 */
import assert from "node:assert/strict";
import test from "node:test";

import { type EIP1193RequestFn, custom } from "viem";

import { SciAaClient } from "../src/client.js";

const PK = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80" as const;
const ACC0 = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266" as const;
const ACC1 = "0x70997970C51812dc3A010C7d01b50e0d17dc79C8" as const;

/** Builds a viem transport whose `request` is the provided handler. */
function mockTransport(handler: (method: string, params: readonly unknown[]) => unknown) {
  const request = (async ({ method, params }) =>
    handler(method, (params ?? []) as readonly unknown[])) as EIP1193RequestFn;
  return custom({ request });
}

// The case-1 golden raw from the Rust sci-aa-txgen (same params as below).
const GOLDEN_CASE1_RAW =
  "0x76f87082a41180830f4240843b9aca00830186a0d8d79470997970c51812dc3a010c7d01b50e0d17dc79c80180c0808001a006732b4d9a231a2baed8d3ff174b534574d3c3bd05d0febb08daf6f9bb89a637a0714c0bca7348f949a791fe115c5e64d276d4b102121593bc20c128abf2d38169";

test("send() submits the encoder's exact raw bytes", async () => {
  let submitted: unknown;
  const transport = mockTransport((method, params) => {
    if (method === "eth_sendRawTransaction") {
      submitted = params[0];
      return "0xe75e30a432742a6f8d1e9206aa9889c89761ed172d228dd888dd346dc5bdb93e";
    }
    throw new Error(`unexpected RPC: ${method}`);
  });
  const client = new SciAaClient({ transport, privateKey: PK, chainId: 42001 });

  const hash = await client.send({
    calls: [{ to: ACC1, value: 1n, input: "0x" }],
    nonce: 0,
    gasLimit: 100_000n,
    maxFeePerGas: 1_000_000_000n,
    maxPriorityFeePerGas: 1_000_000n,
  });

  assert.equal(submitted, GOLDEN_CASE1_RAW, "submitted raw must equal the golden encoding");
  assert.equal(hash, "0xe75e30a432742a6f8d1e9206aa9889c89761ed172d228dd888dd346dc5bdb93e");
});

test("prepare() auto-fills nonce from eth_getTransactionCount", async () => {
  const transport = mockTransport((method) => {
    if (method === "eth_getTransactionCount") return "0x7";
    throw new Error(`unexpected RPC: ${method}`);
  });
  const client = new SciAaClient({ transport, privateKey: PK, chainId: 42001 });

  const prepared = await client.prepare({
    calls: [{ to: ACC1, value: 1n, input: "0x" }],
    gasLimit: 100_000n,
    maxFeePerGas: 1n,
    maxPriorityFeePerGas: 1n,
  });
  assert.equal(prepared.nonce, 7);
});

test("prepare() enforces feePayer === root (matches the handler)", async () => {
  const transport = mockTransport((method) => {
    throw new Error(`unexpected RPC: ${method}`);
  });
  const client = new SciAaClient({ transport, privateKey: PK, chainId: 42001 });
  const calls = [{ to: ACC1, value: 1n, input: "0x" as const }];

  await assert.rejects(
    client.prepare({ calls, feePayer: ACC1, root: ACC0 }),
    /feePayer must equal root/,
  );
  await assert.rejects(
    client.prepare({ calls, feePayer: ACC1 }),
    /feePayer requires root/,
  );
});

test("prepare() auto-fills maxFeePerGas as baseFee*2 + priority", async () => {
  const transport = mockTransport((method) => {
    if (method === "eth_getTransactionCount") return "0x0";
    if (method === "eth_getBlockByNumber") return { baseFeePerGas: "0x3b9aca00" }; // 1 gwei
    throw new Error(`unexpected RPC: ${method}`);
  });
  const client = new SciAaClient({ transport, privateKey: PK, chainId: 42001 });

  const prepared = await client.prepare({
    calls: [{ to: ACC1, value: 1n, input: "0x" }],
    gasLimit: 100_000n,
    maxPriorityFeePerGas: 2_000_000_000n, // 2 gwei
  });
  // baseFee(1g)*2 + prio(2g) = 4 gwei
  assert.equal(prepared.maxFeePerGas, 4_000_000_000n);
  assert.equal(prepared.maxPriorityFeePerGas, 2_000_000_000n);
});

test("registerKey() sends a root-unset AA tx batching authorizeKey [+ bindKey]", async () => {
  let submitted: `0x${string}` | undefined;
  const transport = mockTransport((method, params) => {
    if (method === "eth_getTransactionCount") return "0x0";
    if (method === "eth_sendRawTransaction") {
      submitted = params[0] as `0x${string}`;
      return "0x" + "11".repeat(32);
    }
    throw new Error(`unexpected RPC: ${method}`);
  });
  // Client is constructed with the ROOT key (registration is a root op).
  const client = new SciAaClient({ transport, privateKey: PK, chainId: 42001 });

  await client.registerKey({
    keyId: ACC1,
    restrictions: {
      expiry: 2n ** 64n - 1n,
      enforceLimits: false,
      limits: [],
      allowAnyCalls: true,
      allowedCalls: [],
    },
    agentId: `0x${"ab".repeat(32)}`,
    gasLimit: 500_000n,
    maxFeePerGas: 1n,
    maxPriorityFeePerGas: 1n,
  });

  assert.ok(submitted, "a raw tx must be submitted");
  // 0x76 type byte, and root is encoded empty (0x80) — the batch executes as the signer.
  assert.equal(submitted.slice(0, 4), "0x76");
});
