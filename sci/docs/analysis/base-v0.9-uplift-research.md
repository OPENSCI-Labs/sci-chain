# Base v0.8.0 → v0.9.0 Uplift Research

**Status:** working notes for the `chore/base-v0.9-uplift` PR and the subsequent
`feat/p0-1-keychain` rebase. Gitignored (per CLAUDE.md Rule #7).

**Reference revisions:**
- SCI fork point: `3049ce2e3 fix: error severity for invalid FC state -> reset (#2551)` on Base v0.8.0
- Base v0.9.0 release HEAD: `5f1d968fa fix(proposer): discard proofs with invalid signers (backport #2693 to v0.9.0) (#2697)` on `base-upstream/releases/v0.9.0`
- SCI keychain branch HEAD: `c365d5a65 sci: harden pre-execution hook and document keychain semantics` on `feat/p0-1-keychain`

## 1. Headline findings

1. Base v0.9 **stays on revm 34.0.0 and alloy-evm 0.27.2** — same major versions as v0.8. The Tempo B1 wrapper layer (revm-34-targeted) is unaffected.
2. Base v0.9 performs a **whole-crate Op\* → Base\* rename** across `base-common-evm`: `OpHandler`, `OpContextTr`, `OpTransaction`, `OpHaltReason`, `OpSpecId` all renamed. `Context::op()` → `Context::base()`. `build_op_with_inspector()` → `build_base().with_inspector(...)`.
3. The EVM construction location moved: the **5 handler instantiations our SCI patch swaps to `SciHandler::new()` are no longer in `crates/common/evm/src/api/exec.rs`** — `exec.rs` has been shrunk to a trait-alias module. They now live in `crates/common/evm/src/evm.rs` at lines 224 / 235 / 273 / 308 / 338.
4. `BaseHandler` (v0.9's replacement for `OpHandler`) **still exposes `validate_against_state_and_deduct_caller`** at `handler.rs:102` — same signature and role. Our wrapper pattern survives intact.
5. `crates/common/evm/src/handler.rs` is a **new file** in v0.9 — Base's own `BaseHandler` lives there. Our `sci_handler.rs` is also a new file. Two distinct modules; no name collision.
6. Workspace deps churned but `revm`, `alloy-evm`, and `reth` versions did not move. Many `base-*` crates relocated (e.g., `crates/client/metering` → `crates/execution/metering`), but `base-common-evm` stayed put.
7. The devnet env file received substantial additions (L2 bootnode identities, doubled metering limits, V1→Azul rename) that need to be merged alongside our `L2_CHAIN_ID=42001` override.

## 2. Files Touched by SCI and How v0.9 Affects Them

### 2.1 `crates/common/evm/src/sci_handler.rs` (SCI-owned, new file in v0.8 patch)

| v0.8 symbol | v0.9 symbol | Source |
|---|---|---|
| `OpHandler<EVM, ERROR, FRAME>` | `BaseHandler<EVM, ERROR, FRAME>` | `base_common_evm::handler::BaseHandler`, also re-exported as `base_common_evm::BaseHandler` (lib.rs:25 in v0.9) |
| `OpContextTr` | `BaseContextTr` | `base_common_evm::api::exec` (re-exported at crate root) |
| `OpTransaction<TxEnv>` | `BaseTransaction<TxEnv>` | `base_common_evm::transaction::core::BaseTransaction` |
| `OpTransactionError` | `BaseTransactionError` | `base_common_evm::transaction::error` |
| `OpHaltReason` | `BaseHaltReason` | `base_common_evm::result` |
| `OpSpecId` | `BaseSpecId` | `base_common_evm::spec` |
| `DEPOSIT_TRANSACTION_TYPE` | (unchanged) | still re-exported from `base_common_evm` crate root via `transaction/mod.rs:13` |

`BaseHandler` definition (v0.9 `handler.rs:38-47`):

```rust
pub struct BaseHandler<EVM, ERROR, FRAME> {
    pub mainnet: MainnetHandler<EVM, ERROR, FRAME>,
}

impl<EVM, ERROR, FRAME> BaseHandler<EVM, ERROR, FRAME> {
    pub fn new() -> Self {
        Self { mainnet: MainnetHandler::default() }
    }
}
```

v0.8's `OpHandler` wrapped `MainnetHandler` similarly, just via the `op-revm` external crate. The shape — and the `Handler` trait impl that exposes `validate_against_state_and_deduct_caller` — is API-compatible. Our `SciHandler` wrapper pattern transposes 1:1 by swapping the symbol.

### 2.2 `crates/common/evm/src/api/exec.rs` — the surprise

v0.8 content: ~190 lines, contains 5 `OpHandler::<_, _, EthFrame<EthInterpreter>>::new()` instantiations.

v0.9 content: ~40 lines, contains only `BaseContextTr` trait alias and `BaseError<DB>` type alias. **None of the 5 handler instantiations live here anymore.**

Concrete v0.9 `exec.rs` (`base-upstream/releases/v0.9.0:crates/common/evm/src/api/exec.rs`):

```rust
pub trait BaseContextTr:
    ContextTr<
        Journal: JournalTr<State = EvmState>,
        Tx: BaseTxTr,
        Cfg: Cfg<Spec = BaseSpecId>,
        Chain = L1BlockInfo,
    >
{}

impl<T> BaseContextTr for T where
    T: ContextTr<
            Journal: JournalTr<State = EvmState>,
            Tx: BaseTxTr,
            Cfg: Cfg<Spec = BaseSpecId>,
            Chain = L1BlockInfo,
        >
{}

pub type BaseError<DB> = EVMError<<DB as Database>::Error, BaseTransactionError>;
```

**Implication for SCI patch:** drop the 5 swap-site edits from `exec.rs` entirely. The new home for them is `evm.rs`.

The 2 trait bounds our v0.8 patch added to `OpContextTr` (`Db: alloy_evm::Database`, `Journal: Debug`) need re-examination. The `BaseContextTr` body above does not bind `Db: Database` directly, but the EVM construction in `evm.rs` will. If our `EvmInternals` construction in the hook still needs the bound, add it on the `BaseContextTr` consumer side in `evm.rs` rather than on the trait definition.

### 2.3 `crates/common/evm/src/evm.rs` — new SciHandler swap-site home

5 `BaseHandler::<_, _, EthFrame<EthInterpreter>>::new()` instantiations to swap to `SciHandler::new()`:

| Method | Line | Context |
|---|---|---|
| `transact_one` | 224 | impl ExecuteEvm for BaseEvm |
| `replay` | 235 | impl ExecuteEvm for BaseEvm |
| `inspect_one_tx` | 273 | impl InspectEvm for BaseEvm |
| `system_call_one_with_caller` | 308 | impl SystemCallEvm for BaseEvm |
| `inspect_one_system_call_with_caller` | 338 | impl InspectSystemCallEvm for BaseEvm |

All 5 use the same calling shape: `let mut h = BaseHandler::<_, _, EthFrame<EthInterpreter>>::new();` → swap `BaseHandler` for `SciHandler`. Identical to the v0.8 pattern, just at a different file path.

### 2.4 `crates/common/evm/src/factory.rs` — builder reshuffle

v0.8 (our patch swaps a let-binding into the middle):

```rust
BaseEvm {
    inner: Context::op()
        .with_db(db).with_block(input.block_env).with_cfg(input.cfg_env)
        .build_op_with_inspector(NoOpInspector {})
        .with_precompiles(PrecompilesMap::from_static(
            BasePrecompiles::new_with_spec(spec_id).precompiles(),
        )),
    inspect: false,
}
```

v0.9 (`factory.rs:30-46`):

```rust
Context::base()
    .with_db(db).with_block(input.block_env).with_cfg(input.cfg_env)
    .build_base()
    .with_inspector(NoOpInspector {})
    .with_precompiles(PrecompilesMap::from_static(
        BasePrecompiles::new_with_spec(spec_id).precompiles(),
    ))
```

Two structural shifts:
1. `Context::op()` → `Context::base()`, `build_op_with_inspector(insp)` → `build_base().with_inspector(insp)`.
2. The outer `BaseEvm { inner: ..., inspect: false }` wrapping struct is gone — the builder chain returns the EVM directly.

**SCI patch shape stays the same**: bind `PrecompilesMap::from_static(...)` to a `let mut precompiles = ...;`, call `sci_precompiles::install(&mut precompiles, &input.cfg_env)`, then thread `precompiles` into `.with_precompiles(precompiles)`. Both `create_evm` and `create_evm_with_inspector` get the same edit.

### 2.5 `crates/common/evm/src/lib.rs`

v0.9 added `mod handler; pub use handler::{BaseHandler, IsTxError};` (lib.rs:25-26). Our SCI patch adds `mod sci_handler; pub use sci_handler::SciHandler;`. **No symbol collision** — `mod handler` and `mod sci_handler` are distinct module names. Place our patch next to v0.9's new `mod handler;`.

Also: many of the existing v0.8 exports got renamed (Op*→Base*). These are Base's own changes, not SCI's — they survive the merge automatically.

### 2.6 `crates/common/evm/Cargo.toml`

v0.9 added `base-common-precompiles.workspace = true` in the `# base` section and reorganized feature flags. Our SCI patch adds one line: `sci-precompiles.workspace = true`. Place it in the `# base` section near `base-common-precompiles`. Sort order is waterfall-by-line-length per Base style rule; our line is the longest in that group so it sorts to the end.

Removed deps: `strum` (no longer needed in v0.9). Doesn't affect us.

### 2.7 Workspace `Cargo.toml`

v0.9 reshuffled many `base-*` crate paths (notably `crates/client/*` → `crates/execution/*`) and added new ones (`base-common-genesis`, `base-common-precompiles`). All our `sci/crates/*` entries live in a disjoint namespace and re-insert cleanly:

- `workspace.members`: append our 4 `sci/crates/*` paths.
- `workspace.dependencies`: re-add the 4 entries with their `package = "..."` aliases (`sci-precompiles`, `sci-precompiles-macros`, `sci-precompile-abi`, `tempo-chainspec-shim`).

Also bumped: `alloy-rpc-types-trace` removed. We don't use it. No SCI-side impact.

### 2.8 `etc/docker/devnet-env`

v0.9 added a lot to this file (preserving our `L2_CHAIN_ID=42001` override):

| Addition (Base v0.9) | Implication for SCI |
|---|---|
| L2 bootnode identities block (`L2_BOOTNODE_DOCKER_SUBNET`, `L2_EL_BOOTNODE_*`, `L2_CL_BOOTNODE_*`, advertise IPs for builder/client/seq1/seq2) | Accept verbatim — devnet uses these for op-stack networking |
| `L2_BASE_V1_BLOCK` renamed to `L2_BASE_AZUL_BLOCK`; comment updated | Base rebranded "V1" to "Azul". Our patch doesn't touch this var. |
| Metering limits doubled: 30M→60M gas, 786KB→1.5MB DA, 200ms→1s state root, 2s→5s exec | Accept verbatim — these don't conflict with SCI |
| Removed `CONTENDER_*` vars | Accept removal |
| New `BUILDER_BLOCK_STATE_ROOT_GAS_LIMIT`, `BUILDER_MAX_REJECTED_TXS_PER_BLOCK` | Accept verbatim |

Our patch (`L2_CHAIN_ID=42001`) targets a line that v0.9 does not modify, so the merge of this file is clean — Base's additions land, our chain-id override stays.

## 3. Things NOT changed in v0.9 (relevant for SCI)

- revm pin: `revm = { version = "34.0.0", default-features = false }`
- alloy-evm pin: `alloy-evm = { version = "0.27.2", default-features = false }`
- reth deps still rooted at `paradigmxyz/reth` tag `v1.11.4` (verified via grep)
- `DEPOSIT_TRANSACTION_TYPE` constant still exported from `base_common_evm` crate root
- `BasePrecompiles` and `PrecompilesMap` still in the same place — our `install(...)` call goes in unchanged
- Rust edition (2024), `rust-version`, and `mold` linker setup all unchanged

## 4. Things to watch during step 2 (the rebase)

- `cargo build --release -p based-bin` — confirm the binary crate name is still `based-bin` in v0.9 (likely yes, but the workspace member list moved, so verify).
- The `cargo nextest run -p sci-precompiles` count (307 lib + 14 hook_e2e + 74 macro) should be unchanged. Any test count drift signals a symbol mismatch in our wrapping layer.
- Devnet smoke: with the new metering limits + bootnode identities, the devnet bring-up shape may have shifted. The `sci/devnet/apply-sci-allocs.sh` script (which merges `sci-allocs.json` into op-deployer's `genesis.json`) needs verification that op-deployer's output format hasn't changed in v0.9.

## 5. Out of scope for this uplift

- All Tempo v1.7.1 work (`sci/crates/precompiles/**`, `sci/crates/precompiles-macros/**`, etc.) — separate plan.
- The B1 wrapper-layer design rule in CLAUDE.md — added when Tempo plan lands.
- `feat/p0-2-contracts` (Heath) and `feat/p0-3-gateway` (S) rebases — covered by `base-v0.9-rebase-handoff.md` (written after step 1 lands).
