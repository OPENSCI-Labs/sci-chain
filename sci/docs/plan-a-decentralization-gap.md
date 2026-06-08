# Plan A — Decentralization Gap Analysis (`0x76` local-only → gossipable)

**Status:** analysis + experiment plan. Date: 2026-06-08.
**Branch:** `feat/plan-a-aa-keychain`.
**Driver:** SCI Chain must stay in sync with Base. When Base ships a decentralized
sequencer, SCI must follow. The current Plan A `0x76` AA transaction is **local-only**
(never gossiped, RPC-ingress only), which cannot meet decentralization needs and is a
censorship-resistance / regulatory liability for a payments-oriented L2 at mainnet.

This document is the gap analysis between three reference points — SCI's local-only
`0x76`, Base's native gossipable tx types (Eip1559 / Deposit), and Tempo's fully
gossipable AA tx — and the scoped plan to close it.

---

## Headline

1. **De-local-only is much cheaper than feared.** reth (v1.11.4) gossips and serves
   `GetPooledTransactions` using Base's own `BaseTxEnvelope` / `BasePooledTransaction`,
   **not** alloy's `TxEnvelope` / `PooledTransaction`. `0x76` already satisfies the
   `SignedTransaction` trait wall and the `BaseTxEnvelope → BasePooledTransaction`
   conversion already returns `Ok` for AA. The two `unreachable!()` stubs in the
   alloy-conversion path are needed **only for RPC representation**, not for P2P gossip.
2. **The real hard problem is the L1 forced-inclusion escape hatch.** `0x76` has no
   L1-origin path; the OP-Stack deposit envelope is structurally incompatible with an
   AA tx. A true escape hatch is HIGH effort.
3. **The keychain core is decentralization-agnostic and fully reusable.** No part of the
   keychain hook, batch executor, `fee_payer` sponsorship, or spending-limit metering
   needs to change to make `0x76` gossipable.

---

## 1. De-local-only — small scope

### How a tx becomes gossipable in this codebase (Eip1559 as the positive reference)

- Broadcast (`Transactions` message) sends the **consensus** type: reth
  `transactions/mod.rs:1736-1744` calls `clone_into_consensus()` and broadcasts the
  `BaseTxEnvelope`, gated by `is_broadcastable_in_full()` (default `!is_eip4844()`).
- Serving (`GetPooledTransactions`) calls `clone_into_pooled()`
  (reth `transaction-pool/src/traits.rs:1285-1289`) which does
  `Consensus::try_into::<Pooled>()` = `BaseTxEnvelope → BasePooledTransaction`
  (`crates/common/consensus/src/transaction/pooled.rs:278-284`, delegates to
  `BaseTxEnvelope::try_into_pooled` at `envelope.rs:338`).
- The `propagate` flag (set in `TransactionValidationOutcome::Valid { propagate, .. }`)
  is the runtime gate; the pool's announce/broadcast collectors all
  `filter(|tx| tx.propagate)` (reth `transaction-pool/src/pool/mod.rs:378,416,432,446`).

### Why `0x76` is local-only today (two layers)

1. **Conversion layer (RPC-only):** `BasePooledTransaction::Aa` exists and
   `try_into_pooled` returns `Ok` for AA, but every alloy lowering is stubbed:
   - `into_envelope` → `unreachable!("AA pooled txs ... no ethereum TxEnvelope")`
     (`pooled.rs:104-114`)
   - `From<BasePooledTransaction> for alloy ...PooledTransaction` →
     `unreachable!(... no alloy PooledTransaction")` (`pooled.rs:198-214`)
   - `try_into_eth_pooled` → `Err(...)` (`envelope.rs:360-363`),
     `try_into_eth_envelope` → `Err(...)` (`envelope.rs:381-384`)
2. **Propagation layer (the real guarantee):** the validator force-clears the flag —
   `is_aa = ty() == SCI_AA_TX_TYPE_ID` (`validator.rs:211`) then
   `if is_aa { ... *propagate = false; }` (`validator.rs:224-229`). Admission is
   deliberately NOT origin-gated (comment `validator.rs:204-210`): a
   `--txpool.nolocals` sequencer tags RPC txs `External`, so an origin reject would
   block legitimate RPC ingress. `propagate = false` is the only local-only mechanism.

Because `propagate = false`, the announce/serve path is never exercised, so the two
`unreachable!()` sites never fire today.

### reth v1.11.4 constraint (the key finding)

`NetworkPrimitives::PooledTransaction: SignedTransaction + TryFrom<BroadcastedTransaction>`
(reth `eth-wire-types/src/primitives.rs:51-56`). For Base, `BroadcastedTransaction =
BaseTxEnvelope` and `PooledTransaction = BasePooledTransaction`. **reth does not require
an alloy round-trip** — Base's own pooled type is the network type. `0x76` already
implements `SignedTransaction` (via the `#[derive(TransactionEnvelope)]` macro giving it
`Decodable2718` / `Typed2718` etc.). So pure P2P gossip and `GetPooledTransactions`
serving can work off `BaseTxEnvelope` / `BasePooledTransaction` alone — both of which AA
already satisfies.

### Tempo confirms the pattern

Tempo (v1.7.1) sets `type Pooled = type Consensus = TempoTxEnvelope`
(`crates/transaction-pool/src/transaction.rs:519-520`), with `AA(AASigned)` as a
first-class envelope variant (`envelope.rs:44-64`). There is therefore **no lossy
conversion and no `unreachable!()` anywhere**; `Encodable2718` delegates straight to the
envelope. The validator passes `propagate` through unchanged for AA
(`validator.rs:478,558`) and never gates on origin. This is exactly the shape SCI needs.

### Change set (small)

| Change | Location | Nature |
|---|---|---|
| Remove `*propagate = false` for AA | `crates/execution/txpool/src/validator.rs:224-229` | one-line "off switch" |
| Network-wide registration + hook | `crates/execution/node/src/node.rs:927` (`with_custom_tx_type`) | every peer that ingests AA needs it, else its reth validator rejects `TxTypeNotSupported` |
| DoS field caps (peer-penalization once gossiping) | new; reference Tempo `validator.rs:42-71,201-297` (`MAX_AA_CALLS`, access-list / auth-list caps) | hardening |
| (RPC only) real alloy conversions | `pooled.rs:104,198` | needed for `eth_getTransactionByHash` etc., NOT for gossip |

### Open question to validate FIRST (the minimal experiment)

The static claim "gossip never hits the `unreachable!()` sites" must be confirmed by
tracing the full announce → request → serve → receive → insert path for a
`BasePooledTransaction::Aa`, then by a live two-node devnet test. See §5.

---

## 2. L1 forced-inclusion / escape hatch — HIGH effort

OP-Stack censorship resistance comes from **deposit transactions (`0x7E`) derived from
L1**. The derivation pipeline lives in-repo: `crates/consensus/derive/` (stages,
attributes builder) and `crates/consensus/protocol/src/deposits.rs` (L1
`TransactionDeposited` event → `TxDeposit` via `unmarshal_v0`, version 0 only). Deposits
are injected after the L1-info tx, before pool txs, with `no_tx_pool: true`
(`crates/consensus/derive/src/attributes/stateful.rs:182-208`).

`0x76` has **no** L1-origin path:
- Derivation emits only deposit bytes; no `BaseAaTransaction` / `OpTxType::Aa` anywhere
  in `crates/consensus/`.
- The keychain hook short-circuits deposits (`sci_handler.rs:304-308`) and only runs the
  AA path when `tx_type == 0x76 && aa_parts()` is present.

Structural blockers: `TxDeposit` (`deposit.rs:29-60`) is single-call, unsigned,
L1-authenticated; an AA tx is `calls[]` + session-key-signed + `fee_payer`/`root`. The
deposit decoder has no extension point.

Options (rough effort order):
- **Option 1 (low-medium) — deposit-as-trigger relay.** Force-include a normal deposit
  whose `to` is an L2 relay predeploy that performs the agent action in Solidity (calling
  the keychain precompile + target). Zero derivation / tx-type changes; rides the existing
  censorship-resistant deposit path. Cost: keychain sandbox semantics (fail-fast-no-gas,
  atomic batch deduction) must be reimplemented in Solidity, and `msg.sender` is the
  L1-aliased deposit `from`, not a session key.
- **Option 2 (high) — new deposit version embedding an AA tx.** Changes the
  consensus-critical derivation format; ripples into fault proofs (`crates/proof/`). Not
  recommended.
- **Option 3 (medium-high) — L2 forced-inclusion inbox predeploy** storing signed `0x76`
  blobs replayed later through the real AA path.

Recommended: Option 1 for a PoC; it is the only option needing no derivation changes.

---

## 3. Plan A code — change vs. keep

**Keep (decentralization-agnostic):** keychain hook, `SciHandler` batch executor,
`fee_payer` sponsorship, spending-limit metering, CircuitBreaker, and `BaseAaTransaction`
(10 fields incl. the later-added `root` — note: not 9) with its full RLP / Compact / serde
codec.

**Change (narrow):**
- `validator.rs:224-229` (remove `propagate = false`) + DoS caps — the entire core of
  de-local-only.
- (escape hatch) new relay contract and/or derivation layer, depending on the option.
- (optional, RPC only) `pooled.rs:104,198` `unreachable!` → real alloy conversions.

---

## 4. Must wait for Base

- **The concrete decentralized-sequencer mechanism** (rotating leader / based sequencing /
  shared sequencing) determines whether gossip means "forward to the current proposer" or
  "broadcast across a validator set." De-local-only is a prerequisite for any of them and
  can proceed now; final alignment waits for Base.
- Base/OP's public decentralized-sequencer roadmap (mechanism + timing) needs to be
  verified from primary sources — tracked separately, not assumed.

Note: Tempo's Commonware "subblock broadcast" path is L1-validator-specific and does NOT
transfer to an L2 sequencer model. Only Tempo's eth-wire mempool-gossip pattern transfers.

---

## 5. Minimal validation experiment

**Goal:** confirm "de-local-only = remove `propagate = false` (+ DoS caps)" by proving an
AA `0x76` tx can gossip between two L2 nodes without hitting the `unreachable!()` sites.

**Step 0 — static trace (cheap, no devnet). DONE 2026-06-08 — supports the hypothesis.**
`BasePooledTransaction` derives `TransactionEnvelope` (`pooled.rs:23`) with the `Aa` variant
tagged `#[envelope(ty = 118)]` (`pooled.rs:46`), so the pooled type's `Encodable2718` /
`Decodable2718` are **macro-generated for AA** and delegate to the inner
`Signed<BaseAaTransaction>` — serving/receiving `GetPooledTransactions` never touch the alloy
conversions. The pool↔network conversions route through `into_base_envelope()`
(`pooled.rs:117-125`, AA at `:123`) and `try_into_pooled()` (via `TryFrom<BaseTxEnvelope>`,
`pooled.rs:278-284`, `Ok` for AA). The two `unreachable!()` sites — `into_envelope()` → alloy
`TxEnvelope` (`pooled.rs:104-114`) and `From<BasePooledTransaction>` for alloy
`PooledTransaction` (`pooled.rs:198-214`) — are reachable only when an alloy representation is
explicitly requested (RPC), NOT by gossip. Conclusion: removing the `propagate = false`
override alone suffices; no encode/decode AA arm is missing.

**Step 1 — code change. DONE 2026-06-08.** Added env gate `SCI_AA_GOSSIP` (default off =
current local-only) in `crates/execution/txpool/src/validator.rs`: read-once `LazyLock<bool>`
`AA_GOSSIP_ENABLED` + `if is_aa && !*AA_GOSSIP_ENABLED { ... propagate = false }`. Gating
(not deleting) lets one image A/B both behaviors and keeps the production default unchanged.
`cargo check -p base-execution-txpool` clean; `cargo test -p base-execution-txpool --lib` =
43 passed, 0 failed. Remove the gate once the final gossip design lands.

**Step 2 — two-node devnet test. DONE + PASSED 2026-06-08.** On `54.255.70.252`, rebuilt
both EL images (`base-builder`, `base-reth-node`) with the gate; **wipe-genesis redeploy** of a
fresh healthy 2-node chain (`sci/devnet/redeploy.sh`, safe tracked unsafe gap 3-13). Both ELs
ran with `SCI_AA_GOSSIP=1`; base-client's RPC forwarder (`--rollup.sequencer`) was **disabled**
so the only channel from base-client to the sequencer is devp2p gossip. An AA `0x76` tx
(root=None, signer ACC1, via `sci-aa-txgen`) submitted to **base-client (8545, non-sequencer)**
was **mined on the sequencer** (status 0x1, block 0x3f1, receipt type 0x2) — i.e. it propagated
via gossip and was included. **Neither node panicked** (zero `panic`/`unreachable` in logs);
both stayed healthy. This empirically confirms: de-local-only = remove the `propagate = false`
force-clear, and the gossip wire path is safe (no `unreachable!()` hit). See the
`project_devnet_aagossip_experiment` memory for image/rollback tags.

Phase-1 partial run (no-wipe, sequencer-only, pre-Path-2): AA submitted directly to the
sequencer was admitted + mined without panic across multiple txs, but propagation to a peer
could not be observed because the verifier was ~128k blocks behind (stuck at head 750, a known
devnet P2P quirk). Path 2 (fresh healthy chain) was required for the clean proof.

**Negative control. DONE + PASSED 2026-06-08.** base-client recreated with `SCI_AA_GOSSIP`
unset (gate off, forwarder still off). The same AA tx (root=None, ACC1, nonce 1) submitted to
base-client 8545: it entered base-client's pool (pending=1) but the sequencer's pool stayed
empty (pending=0) and after several blocks (head 0x572→0x586) the tx was **never mined**
(receipt null on both nodes, still pending=1 on base-client). With the gate off the AA does not
propagate and, with the forwarder off, has no other path to the sequencer — proving the
`SCI_AA_GOSSIP` gate is the sole cause of the inclusion seen in Step 2, not RPC forwarding or any
other channel. The A/B (gate on → mined; gate off → stuck local) is airtight.
