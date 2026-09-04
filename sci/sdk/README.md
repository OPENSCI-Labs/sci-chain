# @sci-chain/sdk

JS/TS SDK for SCI Chain. It provides:

- the **encoder** for the native account-abstraction transaction type (`0x76`,
  `BaseAaTransaction`) — the JS port of the Rust dev tool `sci/tools/aa-txgen`, producing
  byte-identical raw transactions accepted by `eth_sendRawTransaction`;
- **call builders** (`nativeTransferCall`, `erc20TransferCall`/`erc20ApproveCall`,
  `contractCall`, keychain `authorizeKeyCall`/`revokeKeyCall`/`updateSpendingLimitCall`/
  `setAllowedCallsCall`/`removeAllowedCallsCall`, registry `bindKeyCall`/`unbindKeyCall`,
  `circuitBreakerTripCall`/`circuitBreakerUntripCall`) and the **registration** helper
  `registerAgentKeyCalls` (Option B: authorize a session key, optionally bind an `agentId`);
- **event decoding** (`decodeAgentEvents`) — recovers keychain/registry/circuit-breaker
  events from a receipt's logs;
- **`SciAaClient`** — a viem-backed client that auto-fills nonce/chain-id/fees, signs, and
  submits AA txs, the `registerKey` convenience, plus `getKey`/`getRemainingLimit`/
  `getRemainingLimitWithPeriod`/`getAllowedCalls`/`isTripped` and registry
  `getBinding`/`isBound`/`agentIdOf` view helpers.

## Install

```bash
cd sci/sdk
npm install
npm test        # golden tests vs. sci-aa-txgen
npm run build   # emit dist/
```

## Usage

### Low-level: encode + sign yourself

```ts
import { signAaTransaction } from "@sci-chain/sdk";

const { raw, hash } = await signAaTransaction(
  {
    chainId: 42001,
    nonce: 0,
    maxPriorityFeePerGas: 1_000_000n,
    maxFeePerGas: 1_000_000_000n,
    gasLimit: 100_000,
    calls: [{ to: "0x7099…79C8", value: 1n, input: "0x" }],
    // optional sponsored gas / root delegation:
    // feePayer: "0xf39F…2266",
    // root:     "0xf39F…2266",
  },
  privateKey, // 0x-prefixed session-key hex
);

// submit `raw` via eth_sendRawTransaction
```

### High-level: SciAaClient

```ts
import { http } from "viem";
import { SciAaClient, erc20TransferCall } from "@sci-chain/sdk";

const client = new SciAaClient({
  transport: http("http://localhost:8545"),
  privateKey, // session key
  chainId: 42001,
});

// nonce / fees auto-filled; gasLimit required (0x76 can't use eth_estimateGas)
const hash = await client.send({
  calls: [erc20TransferCall(token, recipient, 50n)],
  gasLimit: 200_000n,
  // root: rootAddr, feePayer: rootAddr,   // agent acting as root, sponsored gas
});
const receipt = await client.waitForReceipt(hash);
```

### Register a session key (Option B)

Registration authorizes the session key on the keychain (and optionally binds an off-chain
`agentId`) in one AA tx. Because it bootstraps the key, it cannot be delegated through `root`
— it must be sent **by the root account itself**, so construct the client with the *root*
key:

```ts
import { SciAaClient } from "@sci-chain/sdk";

const rootClient = new SciAaClient({ transport, privateKey: rootKey, chainId: 42001 });
await rootClient.registerKey({
  keyId: sessionKeyAddress,
  restrictions: { expiry: 2n ** 64n - 1n, enforceLimits: false, limits: [], allowAnyCalls: true, allowedCalls: [] },
  agentId, // optional bytes32 — bound in the AgentAccessKeyRegistry
  gasLimit: 500_000n,
});
```

### Decode events from a receipt

```ts
import { decodeAgentEvents } from "@sci-chain/sdk";

const receipt = await client.waitForReceipt(hash);
for (const ev of decodeAgentEvents(receipt.logs)) {
  // ev.eventName ∈ { KeyAuthorized, KeyRevoked, SpendingLimitUpdated, AccessKeySpend,
  //                  KeyBound, KeyUnbound, Tripped, Untripped }
}
```

## Wire format

```
signing payload = 0x76 ‖ rlp([chainId, nonce, maxPriorityFeePerGas, maxFeePerGas,
                               gasLimit, calls, accessList, feePayer, root])
signing hash    = keccak256(signing payload)
signed tx       = 0x76 ‖ rlp([…the 9 fields, yParity, r, s])
```

`calls` is a list of `[to, value, input]`; `feePayer`/`root` encode as the 20-byte
address or the empty string (`0x80`) when absent; the empty access list is `0xc0`.
Mirrors `crates/common/consensus/src/transaction/aa.rs`.

## Scope

- ✅ Encode + sign `0x76` transactions (`aa.ts`).
- ✅ Call builders for native/ERC-20/keychain/registry/circuit-breaker (`calls.ts`).
- ✅ Agent registration helper (Option B: `registerAgentKeyCalls` / `SciAaClient.registerKey`).
- ✅ Event decoding (`decodeAgentEvents`, `events.ts`).
- ✅ `SciAaClient`: auto-fill + sign + submit + receipt + keychain/registry view reads
  (incl. `getAllowedCalls`, `getRemainingLimitWithPeriod`) (`client.ts`).
- ⬜ MPP / HTTP-402 gateway integration — only if pay-per-use access becomes the product model.

Gas note: `eth_estimateGas` does not understand type `0x76`, so `gasLimit` must be supplied
(or the client's `defaultGasLimit` is used).
