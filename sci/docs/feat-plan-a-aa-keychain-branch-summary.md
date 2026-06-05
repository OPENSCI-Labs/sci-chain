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
  deleted. (Plan A hook coverage is the `sci/devnet/e2e/` live loop.)
- Docs: CLAUDE.md, audit-scope, registration-decision, and the AA e2e runbook updated.

## 7. Verification

- `cargo check`: `sci-precompile-abi`, `sci-precompiles`, `base-common-evm`, and the full `base`
  binary — all pass.
- `cargo test -p sci-precompiles`: 315 passed, 0 failed, 1 ignored.
- `forge build` + `forge test`: 16 unit tests pass; 7 integration suites skip without a fork.
- All Plan A Phase 1–6 flows devnet-verified (per the phase tracker in
  `docs/test/plan-a-status.md`, gitignored).

## 8. One-sentence summary

`feat/plan-a-aa-keychain` adds a native AA transaction type (`0x76`) with a pre-execution
keychain hook on top of the Tempo-ported `AccountKeychain` precompile, delivering the Agent
permission sandbox without EIP-7702 — and removes the superseded Plan B delegator path entirely.
