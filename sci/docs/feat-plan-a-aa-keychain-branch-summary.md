# feat/plan-a-aa-keychain — Branch Summary

**Branch:** `feat/plan-a-aa-keychain` (based on Base Azul **v0.9.0**, `64efe0995`).
**Supersedes:** the `feat/p0-1-keychain` / `feat/p0-2-contracts` line (whose own summaries this
document replaces). **Status:** Plan A Phases 0–6 implemented and devnet-verified; Plan B
(EIP-7702 + `SCIAgentDelegator`) fully removed.

This branch turns SCI Chain into an Agent-native L2 in two layers:

1. **Keychain layer** (ported from Tempo v1.7.1) — the `AccountKeychain` precompile at
   `0xAAAA..00`, the SCI-only `SciAgentState` circuit-breaker precompile at `0xAAAA..01`, the
   3 agent predeploys (`0xBBBB..01/02/03`), and the compat shim crates that let verbatim Tempo
   source compile against Base's revm 34.
2. **Agent-tx carrier — Plan A** — a **native AA transaction type `0x76`** (`BaseAaTransaction`)
   carrying `calls[]` + optional `root` + `fee_payer`, with a Rust pre-execution hook
   (`run_aa_keychain_hook`: CircuitBreaker → Scope → SpendingLimit) applied before execution.
   This replaces the earlier Plan B design (standard tx + EIP-7702 delegation to a delegator
   predeploy + `run_pre_execution_hook`), which has been removed from this branch.

## 0. Commit log (grouped)

```
# Keychain port + scaffold + contracts + devnet tooling
a119bc77c sci: scaffold sci/ workspace
05acf216d sci: port keychain precompile + wire pre-execution hook
036e70d16 sci: port Tempo v1.7.1 keychain (T5 witness) via revm-shim compat crate
36e33d85a contracts: P0-2 agent contracts + Foundry project
41d469a31 devnet: SCI predeploy alloc tooling + E2E walkthrough

# Plan A — AA tx type 0x76 (PoC → Phase 1 → Phase 2)
8d46f367c base-mod: minimal SCI AA transaction type (PoC, 0x76)
6d40feb96 base-mod: wire SCI AA tx into BaseTxEnvelope (~40 match arms)
4256423fe base-mod: execute AA tx in the EVM/proof path
30754234e base-mod: AA tx DB codecs (reth Compact + bincode)
4a479eeb0 base-mod: pool AA txs local-only
7f59dd923 base-mod: register AA tx type with reth validator + aa-txgen tool
ac3fa25b9 sci: add root field (Phase 2 identity model)
dceb456d1 base-mod: AA multi-call atomic executor (2a)
4c02cc702 base-mod: fee_payer sponsored gas (2b)
46cc3fd24 sci: AA keychain authorization gate (2c)
50a7256e3 sci: AA spending-limit meter (2c-ii)
23afb504e base-mod: serialize AA numeric fields as hex quantities (batcher fix)
954c6600f / f701277ee / 177f7d523  base-mod: fundless-signer pool admission, tracing parity, block-build execution

# Phase 4–6
1610a8a77 base-mod: local CPU compressed-proof branch for `multi`
b0185971e contracts: adapt agent contracts to Plan A AA model (§6.2)
c9f88474d contracts: agent registration path Option B (drop IDA NFT stub)
e87acb383 sci: Phase 6 docs — AA-flow E2E runbook + audit scope
25c485a92 sci-fix: keychain-hook AA rejection must be InvalidTransaction, not Custom

# Working tree (this change, pending commit): Plan B removal + e2e scripts + Blockscout shim
```

## 1. Base file modifications

Two tracked groups (full detail + justification in CLAUDE.md "Critical Rules" #2).

**Group A — keychain precompile integration** (6 modified + 1 new; kept intentionally small):
`Cargo.toml`, `crates/common/evm/{Cargo.toml,factory.rs,lib.rs,evm.rs}`,
**`crates/common/evm/src/sci_handler.rs` (new)**, `etc/docker/devnet-env`.
`SciHandler` wraps Base's `BaseHandler`, overriding `validate_against_state_and_deduct_caller`
(AA agent-tx authorization) and `execution_result` (deferred spending-limit deduction).

**Group B — Plan A AA tx type `0x76`** (a native tx type must thread through Base's shared
envelope and every variant match site):
`crates/common/consensus/src/transaction/aa.rs` (new `BaseAaTransaction` + `Call`),
`transaction/{envelope,typed,tx_type,pooled,core}.rs`, `reth_compat.rs`, `{lib,transaction/mod}.rs`;
`crates/common/rpc-types/src/transaction/request.rs`;
`crates/execution/evm/src/receipts.rs`, `crates/execution/flashblocks/src/receipt_builder.rs`,
`crates/execution/txpool/src/{validator,transaction}.rs`, `crates/execution/node/src/node.rs`.

## 2. SCI Rust crates (`sci/crates/`, `sci/tools/`)

- `precompiles/` — `AccountKeychain` (Tempo v1.7.1, verbatim + documented SCI patches), the
  SCI-only `SciAgentState` CB precompile, and the **AA-native pre-execution hook**
  (`handler/{mod,hook,decode}.rs`: `run_aa_keychain_hook` + `apply_aa_post_execution_deductions`
  + `classify_token_call`). `install()` registers the keychain precompile.
- `precompiles-macros/` — `#[contract]` / `#[derive(Storable)]` proc macros (verbatim Tempo).
- `precompile-abi/` — `IAccountKeychain` bindings + ERC-20/SCI-20 selectors (`predeploys/erc20`).
- `revm-shim/` — exposes the revm-38-shape `PrecompileOutput`/`PrecompileHalt`/state-gas API on
  top of Base's revm 34 (scoped to `sci-precompiles` only).
- `tempo-chainspec-shim/` — `SciHardfork`/`TempoHardfork` ladder.
- `sci/tools/aa-txgen/` — dev tool that builds/signs/2718-encodes a SCI `0x76` tx for
  `eth_sendRawTransaction` (cast cannot construct the custom type).

## 3. SCI contracts (`sci/contracts/`)

- `src/agent/` — `AgentAccessKeyRegistry`, `AgentBudgetController`, `AgentCircuitBreaker`
  (predeploys at `0xBBBB..01/02/03`).
- `src/interfaces/` — public interfaces (`IAccountKeychain`, `ISciAgentState`, the 3 agent ifaces).
- `script/Deploy.s.sol` — smoke-deploys the 3 agent predeploys.
- `test/` — unit tests (Registry/Budget/CircuitBreaker) + fork-based `test/integration/` suite
  (skips without `--fork-url`); the live agent-loop is `sci/devnet/e2e/` (AA txs can't be built
  in Foundry).
- **Registration model: Option B** (`agent-registration-path-decision.md`) — ERC-8004 one-step
  registration done by the root directly calling `keychain.authorizeKey` (+ optional
  `registry.bindKey`); no on-chain IDA NFT, no ERC-6551 TBA.

## 4. SCI devnet (`sci/devnet/`)

- `sci-allocs.json` (precompile `0xef` markers) + `sci-predeploy-allocs.json` (3 predeploys) +
  `apply-sci-allocs.sh` / `apply-predeploy-allocs.sh` / `export-predeploy-allocs.sh`.
- `docker-compose.sci.yml` — SCI image overrides.
- `e2e/` — `e2e-loop.sh` (P1–P5 agent loop) + `reject-test.sh` (hook-reject must not wedge) + README.
- `redeploy.sh` — clean wipe-genesis redeploy (back-to-back stages; no CL restarts).
- `blockscout/rpc-shim.py` — presents AA (`0x76`) txs as EIP-1559 (`0x2`) so stock Blockscout
  v7.0.2 can index them (see `docs/test/plan-a-blockscout-aa-rpc-options.md`).

## 5. Phase status (Plan A)

| Phase | Scope | Status |
|---|---|---|
| 0 | PoC: decode → execute → proof for a minimal `0x76` | ✅ Go/No-Go passed |
| 1 | full tx type (Compact/bincode codecs, local-only mempool, validator registration) | ✅ devnet-verified |
| 2 | handler: 2a multi-call executor / 2b fee_payer sponsored gas / 2c keychain gate + limit meter | ✅ devnet-verified |
| 4 | multi-proof: SP1 CPU **execute** proves AA + keychain blocks (keychain ≈ +0.5M cycles); real compressed proof needs GPU | ✅ provability shown |
| 5 | P0-2 contracts adapted to AA model; Option B registration | ✅ |
| 6 | agent-loop e2e (register → transfer → limit → breaker → expiry); reject-doesn't-wedge | ✅ full P1–P5 green, no wedge |

## 6. Plan B removal (this branch)

Plan B (the EIP-7702 + delegator agent path) is fully removed — it was a no-op dead path under
Plan A (its `run_pre_execution_hook` only fired on a 7702 delegation to the delegator address,
which Plan A never creates):

- Contracts: `SCIAgentDelegator.sol`, `ISCIAgentDelegator.sol`, `SciAgentRegistrar.sol` deleted;
  `0xCCCC01` predeploy removed from genesis allocs + export script.
- Rust: `run_pre_execution_hook`, `decode_execute_batch`/`InnerCall`, the `ISCIAgentDelegator` ABI
  (`sci_agent_delegator.rs`), and the Plan B deduction path removed; `SciHandler`'s non-agent
  branch now falls straight through to normal execution.
- Tests: the 14 Plan B `hook_e2e.rs` integration tests + the delegator/registrar unit tests
  deleted. (Since restored AA-natively — see §7.)
- Docs: CLAUDE.md, audit-scope, registration-decision, and the AA e2e runbook updated.

## 7. Code-review hardening round (2026-06-10 review → 2026-06-11 fixes)

A full-branch review (working notes: `docs/test/plan-a-code-review-2026-06-10.md`, gitignored,
with a per-finding resolution table at the bottom) found **no consensus or fund-safety
criticals** but six Medium findings; all six plus most Low findings were fixed in
`37d01c989..6e5e284b1`:

- **M-1** — the D-gas `address(0)` sentinel is charged even when a sponsored
  (`fee_payer == root`) batch **reverts**; token/native-value deductions stay success-only
  (strong-R1). Previously a deliberately-reverting batch burned root's ETH for gas without
  ever touching the quota.
- **M-2** — `ERC20.transferFrom(from == root, …)` is metered against the per-token quota
  (the batch runs with `msg.sender == root`, so root IS the spender).
- **M-3** — pool admission gate for sponsored AA txs (`check_aa_keychain_authorization`):
  `fee_payer` must equal `root`, and `keys[root][signer]` must be a plausibly-active
  keychain record (raw state-slot read; helpers in `account_keychain/sci_ext.rs`). Kills
  zero-cost pool stuffing via fresh unfunded signers. **Behavior change:** a sponsored
  `0x76` tx is rejected at RPC admission until its `authorizeKey` is mined.
- **M-4** — `sci-predeploy-allocs.json` re-exported (stale since `41d469a31`; a fresh
  genesis shipped a BudgetController whose `gasBudget()`/`GAS_TOKEN()` reverted).
- **M-5** — registry bindings keyed by `(account, keyId)` instead of a squattable global
  `keyId`; view functions + SDK take an extra `account` param; `bindKey` rejects expired
  keychain keys.
- **M-6** — `tests/hook_e2e.rs` restored as **13 AA-native integration tests**
  (authorization/CB/scope rejections, strong-R1, M-1/M-2 regressions, batch atomicity);
  the previously vacuous fork invariant suite now drives a real guardian handler with a
  ghost model. The restored suite immediately caught (and the round fixed) a live bug:
  `execute_aa_batch` never journal-loaded `root`, so a value-bearing call with `root` set
  and no `fee_payer` panicked inside revm's `transfer_internal`.
- Low sweep: empty-batch admission reject (L-2), `InMemorySize` undercount (L-3), hook
  system-error propagation + `KeyTripped` business error instead of `Fatal` (L-5/L-6),
  missing-sentinel-row rejection documented as intentional (L-7), `SCI_LAUNCH_HARDFORK`
  constant unifying hook/install at T5 (L-8), SDK address-field validation + RLP zero
  coercion (L-10), `renounceOwnership` disabled on the circuit breaker (L-12 part).

Still tracked: L-1 (`pooled.rs` alloy-lowering `unreachable!` arms), L-11 (SDK golden
vectors for access-list/CREATE need `aa-txgen` flags), the L-12 remainder
(`checkAndAlert` level-trigger, mock permissiveness), and a devnet genesis rebake
(`deploy-fresh.sh`) to pick up the new predeploy bytecode.

## 8. Verification

- `cargo check`: `sci-precompile-abi`, `sci-precompiles`, `base-common-evm`,
  `base-common-consensus`, `base-execution-txpool`, and the full `base` binary — all pass.
- `cargo test -p sci-precompiles --lib`: 318 passed, 0 failed, 1 ignored.
- `cargo test -p sci-precompiles --test hook_e2e`: 13 passed (AA-native suite).
- `cargo test -p base-execution-txpool --lib`: 46 passed (incl. the AA admission gate).
- `forge build` + `forge test`: 19 unit tests pass; 7 integration suites skip without a fork.
- SDK (`sci/sdk`): 27 tests pass; golden byte-parity with `sci-aa-txgen` unchanged.
- All Plan A Phase 1–6 flows devnet-verified (per the phase tracker in
  `docs/test/plan-a-status.md`, gitignored).
- **Full post-review redeploy verified on the GPU devnet (2026-06-11):** images rebuilt
  from this branch on the box, fresh genesis via `deploy-fresh.sh` (M-4 allocs baked —
  `GAS_TOKEN()`/`gasBudget()` resolve), then:
  - `p1-p5-integration.sh`: **27/27 PASS** — every rejected case behaves correctly under
    the M-3 admission gate (sponsored sends wait for `authorizeKey` inclusion).
  - M-3 verified live: an unauthorized sponsored `0x76` is rejected at
    `eth_sendRawTransaction` with "no active keychain key for root …" (never pooled).
  - forge integration (fork, `CHECK_BYTECODE_PARITY=1`): **45/45 PASS** — DeploymentParity
    3/3 byte-match (the 2026-06-10 drift is gone) and the Invariants suite runs with
    `reverts: 0` (ghost model actually driving state).

## 9. TBD — agent-facing tooling (send AA txs without `sci-aa-txgen`)

Today an agent can only build a `0x76` AA tx via the dev CLI `sci-aa-txgen`. Standard
wallets / ethers / viem don't recognize the custom tx type, and SCI's `0x76` is
wire-incompatible with Tempo's (9 vs 14 fields), so Tempo's viem/SDK/cast can't be reused.
Tempo, by contrast, lets an agent simply call an MCP tool `send_payment(to, amount, memo)`
because the encoding is baked into its ecosystem (`agent → tempo-mcp → tempo-accounts-sdk →
viem → 0x76`; verified against `~/opensci/tempo-test-net/plan-b/logs/audit.jsonl`).

To give SCI agents the same "just send" experience, three layers remain to be built
(bottom-up):

1. **A JS encoder for SCI's `0x76`** — either fork viem to add the SCI AA tx type, or
   re-implement `aa-txgen`'s `BaseAaTransaction` encoding in TypeScript. (SCI's format
   differs from Tempo's, so viem's built-in `tempo` support cannot be reused directly.)
2. **An SDK layer** — wraps keychain management (`authorizeKey` / `revokeKey`) + sponsored
   sending (`fee_payer == root`), analogous to `tempo-accounts-sdk`.
3. **A Gateway / MCP server** — exposes high-level agent tools (e.g. `send_payment`),
   analogous to `tempo-mcp`; this is the `SCI Agent Gateway` (MPP + REST) in the
   architecture. The agent then calls `send_payment(to, amount)` and never touches
   `sci-aa-txgen`.

Reference stack to mirror (clone + archaeology notes under `~/opensci/tempo-test-net/`):
`agent → tempo-mcp → tempo-accounts-sdk → viem → 0x76`.

## 10. One-sentence summary

`feat/plan-a-aa-keychain` adds a native AA transaction type (`0x76`) with a pre-execution
keychain hook on top of the Tempo-ported `AccountKeychain` precompile, delivering the Agent
permission sandbox without EIP-7702 — and removes the superseded Plan B delegator path entirely.
