# Blockscout explorer (devnet) — AA (`0x76`) RPC shim & known issues

The devnet block explorer (Blockscout) cannot natively index SCI's native AA transaction
type (`0x76`, `BaseAaTransaction`). `rpc-shim.py` is a compatibility layer that lets the
stock Blockscout image index AA blocks. This README documents how it works and the
follow-up work needed to display AA txs natively.

## What `rpc-shim.py` does

A tiny stdlib-only Python reverse proxy in front of the L2 RPC. Blockscout's bs-web is
pointed at it (`ETHEREUM_JSONRPC_HTTP_URL` / `ETHEREUM_JSONRPC_TRACE_URL` →
`http://blockscout-rpc-shim:8545`). For every JSON-RPC response it recursively:

- rewrites each transaction object with `type: "0x76"` → `type: "0x2"`, and
- lifts `gas` (from `gasLimit`), `to`, `value`, `input` out of `calls[0]` to the top level.

Non-AA txs and receipts (already `type 0x2`) pass through untouched.

### Why it is needed

Stock Blockscout v7.0.2's `EthereumJSONRPC.Transaction.do_elixir_to_params/1` has **no
`type` guards** — it matches purely on key presence. Its EIP-1559 clause requires
`gas`/`input`/`value` at the top level. SCI's AA tx RPC JSON instead emits `gasLimit` and
nests `to`/`value`/`input` inside `calls[0]`, so no clause matches → `FunctionClauseError`
→ bs-web crash-loops importing the first AA block, freezing the explorer. The shim makes
the AA tx *look like* an EIP-1559 tx so the indexer accepts it.

---

## KNOWN ISSUE — AA txs are displayed as type 2, not natively as `0x76`

**Status:** accepted workaround on devnet; native support is future work.

The on-chain reality is unchanged — `eth_getTransactionByHash` (direct, not via the shim)
returns `type: 0x76`, the tx is stored and re-derived as the native AA type, and the
receipt is `type 0x2` (the Plan A AA→EIP-1559 receipt mapping). The explorer, however,
shows these txs as **type 2 (EIP-1559)** because:

1. the shim deliberately rewrites `0x76` → `0x2` for the indexer, and
2. the AA receipt is already mapped to an EIP-1559 receipt in the node.

So "type 2 in Blockscout" is **expected**, not a bug.

### Limitations of the shim approach

- **No native `0x76` display.** Users cannot see that a tx is an agent/AA tx in the UI;
  it is indistinguishable from a normal EIP-1559 transfer.
- **Multi-call AA txs show only `calls[0]`** (the first-call approximation), consistent
  with the rest of the Plan A PoC. The full `calls[]` batch, `root`, and `fee_payer`
  fields are not surfaced.
- The shim is devnet tooling, not a production explorer integration.

### Future improvement (to retire the shim)

Native Blockscout support for the `0x76` type, instead of masquerading it as `0x2`:

1. Patch / extend Blockscout's tx param decoder to recognize `0x76` and decode the AA
   shape (`gasLimit`, `calls[]`, `root`, `fee_payer`) directly — display the full batch,
   the root/delegation, and sponsored-gas info.
2. Requires the node's AA RPC JSON to expose the complete AA fields (today it is a
   first-call approximation — see `sci/docs/test/plan-a-status.md` "PoC simplifications").
   Backfilling the full AA RPC representation is a prerequisite.
3. Once native decoding lands, point bs-web back at the L2 RPC directly and remove the
   `blockscout-rpc-shim` service.

Until then, the shim is the pragmatic devnet solution.

---

## Operational notes

- **Wipe the explorer DB on every wipe-genesis redeploy.** Blockscout's postgres volume
  (`blockscout_bs-pgdata`) is independent of the chain. A genesis wipe restarts the L2 at
  block 0, but the old chain's higher-numbered blocks remain in the DB, so the explorer's
  "latest" view shows stale (prior-chain) data. Reset with:

  ```bash
  cd ~/sci-dev/blockscout
  docker compose down
  docker volume rm blockscout_bs-pgdata
  docker compose up -d
  ```

  Use `down`/`up` (not `restart` — restart keeps the volume). This should be folded into
  the wipe-genesis redeploy procedure.

- **Do not `restart` to recover a netless frontend after a host reboot** — see the
  recovery notes (host systemd nginx squatting `:4000`; frontend attached to no docker
  network) tracked alongside the devnet deployment runbook.
