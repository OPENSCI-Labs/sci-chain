# @sci-chain/sdk

JS/TS SDK for SCI Chain. It provides:

- the **encoder** for the native account-abstraction transaction type (`0x76`,
  `BaseAaTransaction`) — the JS port of the Rust dev tool `sci/tools/aa-txgen`, producing
  byte-identical raw transactions accepted by `eth_sendRawTransaction`;
- **call builders** (`nativeTransferCall`, `erc20TransferCall`/`erc20ApproveCall`,
  `contractCall`, keychain `authorizeKeyCall`/`revokeKeyCall`/`updateSpendingLimitCall`,
  `circuitBreakerTripCall`/`circuitBreakerUntripCall`);
- **`SciAaClient`** — a viem-backed client that auto-fills nonce/chain-id/fees, signs, and
  submits AA txs, plus `getKey`/`getRemainingLimit`/`isTripped` view helpers.

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
- ✅ Call builders for native/ERC-20/keychain/circuit-breaker (`calls.ts`).
- ✅ `SciAaClient`: auto-fill + sign + submit + receipt + view reads (`client.ts`).
- ⬜ Agent registration helper (Option B: root calls `authorizeKey`), richer keychain
  reads (`getAllowedCalls`), and event decoding — to be layered on incrementally.

Gas note: `eth_estimateGas` does not understand type `0x76`, so `gasLimit` must be supplied
(or the client's `defaultGasLimit` is used).
