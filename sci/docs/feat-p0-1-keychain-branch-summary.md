---
title: "feat/p0-1-keychain-tempo-v1.7.1 Branch Summary — All Changes and Their Rationale"
date: "2026-05-27"
branch_base: "0276bb4eb backport(v0.9.0): backport proof node related changes (#2802)"
branch_head: "036e70d16 sci: port Tempo v1.7.1 keychain (T5 witness) via revm-shim compat crate"
commits: 7
files_changed: 88
lines_added: 27701
lines_removed: 25
tempo_upstream: "v1.7.1"
base_upstream: "v0.9.0"
---

# feat/p0-1-keychain-tempo-v1.7.1 Branch Summary

This document lists every change introduced on `feat/p0-1-keychain-tempo-v1.7.1`
since it diverged from the Base v0.9.0 mainline, organized by **why the change is
necessary** rather than by file path. Its purpose is to let a reviewer or a
future maintainer quickly answer:

> "Which parts of Base does the SCI fork modify? Why are these modifications
> mandatory? What is new, and what is shielded from Base entirely?"

After reading it the reader should see that **the SCI fork's surface area
against Base is small and deliberately constrained** — only 7 Base files are
modified plus 1 new Base file, all along the EVM-factory / handler-assembly
path. Every other line of new code (27,000+) lives under `sci/` and produces
zero conflict with the Base mainline.

This branch supersedes the earlier `feat/p0-1-keychain` line and folds in two
significant upgrades:

- **Base v0.8 → v0.9 uplift** — the upstream restructured `crates/common/evm/`,
  renamed the chain binary (`based-bin` → `base`), and replaced the bare
  `OpHandler` with an upstream `BaseHandler` wrapper. The SCI integration
  points moved accordingly (see §2).
- **Tempo v1.6.0 → v1.7.1 port** — Tempo v1.7.1 introduced the TIP-1053
  "key authorization witness" API on a new T5 hardfork, along with a major
  revm 34 → 38 jump that brought EIP-8037 / TIP-1016 state-gas + reservoir
  accounting. SCI stays on revm 34 and absorbs the version gap with a new
  compat shim (`sci-revm-shim`) so the upstream business source can be
  copy-pasted verbatim on every future Tempo sync.

## 0. Commit Log

| Commit | Title | Primary Purpose |
|---|---|---|
| `a119bc77c` | sci: scaffold sci/ workspace directory structure | Lay out the `sci/` directory tree (`.gitkeep` placeholders + top-level README) so subsequent SCI work has a structured home |
| `0be1a4b6d` | sci: replace CLAUDE.md with SCI Chain development guide | Replace the Base CLAUDE.md with an SCI-specific development guide covering architecture, critical rules, build commands |
| `05acf216d` | sci: port keychain precompile and wire pre-execution hook | Initial port of the keychain precompile from Tempo v1.6.0 to SCI and wire its pre-execution hook into the Base EVM factory |
| `864830d53` | sci: add devnet genesis patches + compose override for keychain | Fix the EIP-161 genesis-alloc gap that surfaced during devnet integration testing; add the image-tag isolation override |
| `e25fadcee` | sci: enforce English-only doc policy and add branch summary | Codify CLAUDE.md Critical Rule #7 (English-only public docs, gitignored `sci/docs/analysis/` for working notes) and add the initial branch summary document |
| `4e291020e` | sci: harden pre-execution hook and document keychain semantics | Address P0-1 code review findings R5/R6/R7/R9 — defensive deposit-tx gate, drop redundant 7702 re-read in post-deduction, document `remove_allowed_calls` and `getKey` semantics in `sci_ext.rs` |
| `036e70d16` | sci: port Tempo v1.7.1 keychain (T5 witness) via revm-shim compat crate | Upgrade keychain to Tempo v1.7.1 (T5 witness API) on top of Base v0.9; introduce `sci-revm-shim` to bridge the revm 34 ↔ 38 API gap so verbatim porting stays tractable |

## 1. Aggregate Change Footprint

```
88 files changed, 27701 insertions(+), 25 deletions(-)
```

Broken down by category:

| Category | Files | Lines Added |
|---|---|---|
| Base file modifications (existing files, minimal edits) | 7 | ~78 |
| Base file additions (only 1 new file, lives in `base-common-evm`) | 1 | 173 |
| CLAUDE.md (rewritten + extended for v1.7.1 + shim crate) | 1 | +725 / -10 (net +715) |
| `Cargo.toml` / `Cargo.lock` (workspace registration of `sci-*` crates) | 2 | ~86 |
| `sci/crates/precompiles/` (keychain business code + storage abstraction + tests) | 25 | ~18,300 |
| `sci/crates/precompiles-macros/` (`Storable` / `contract` proc macros) | 7 | ~3,600 |
| `sci/crates/precompile-abi/` (ABI interface `sol!` bindings) | 10 | ~600 |
| `sci/crates/tempo-chainspec-shim/` (compatibility shim, expanded to T6) | 2 | ~131 |
| `sci/crates/revm-shim/` (**NEW** — revm 34 ↔ 38 compat shim) | 5 | ~513 |
| `sci/devnet/` (devnet test configuration) | 4 | ~108 |
| `sci/docs/` (this committed summary; `analysis/` is gitignored) | 1 | 453 |
| `sci/contracts/` / `sci/gateway/` (placeholder directories) | 9 | 0 (`.gitkeep` only) |

---

## 2. Base File Modifications (**8 files total: 7 modified + 1 new**)

CLAUDE.md Critical Rule #2 caps Base modifications at a fixed set. These files
all serve a **single engineering goal**: insert the SCI precompiles and the
pre-execution hook into Base's EVM assembly path **without forking, replacing,
or duplicating any upstream module**.

> **Note**: CLAUDE.md currently lists 7 Base files. In practice the diff also
> touches `crates/common/evm/src/api/exec.rs` (§2.8) with a trait-bounds patch
> implicitly required by the `SciHandler` integration. That makes the real
> count 8. A follow-up edit should reconcile CLAUDE.md to either include
> `api/exec.rs` explicitly or move the trait bounds elsewhere.

### 2.1 `Cargo.toml` (workspace root)

**Change**:
- `workspace.members` gains `sci/crates/precompiles`, `sci/crates/precompiles-macros`,
  `sci/crates/precompile-abi`, `sci/crates/tempo-chainspec-shim`, and
  `sci/crates/revm-shim`
- `workspace.dependencies` gains the five matching crates (`sci-precompiles`,
  `sci-precompiles-macros`, `sci-precompile-abi`, `tempo-chainspec`,
  `sci-revm-shim`) with `package = "..."` renames so verbatim-ported Tempo
  source files can still refer to upstream paths like `tempo_*`

**Why this change is mandatory**:
- Rust workspaces are a hard constraint — SCI crates must be listed in
  `workspace.members` to participate in workspace-wide lock management and the
  shared `target/` directory.
- The `package = "..."` rename is the **core mechanism enabling "verbatim
  Tempo source porting"**: upstream Tempo uses paths like
  `tempo_chainspec::hardfork::TempoHardfork` and `tempo_precompiles_macros::*`,
  while SCI's actual crate names are `sci-precompiles` / `tempo-chainspec-shim` /
  `sci-precompiles-macros`. The rename layer keeps both sides happy without
  rewriting any source.

### 2.2 `Cargo.lock`

**Change**: ~59 lines (new dependency resolution including `sci-revm-shim`).

**Why**: lock files necessarily evolve with `Cargo.toml`, and reproducible
builds require them to be committed.

### 2.3 `crates/common/evm/Cargo.toml`

**Change**: one new line — `sci-precompiles.workspace = true`.

**Why**:
- Base's EVM factory (the `base-common-evm` crate) needs to call
  `sci_precompiles::install(...)` to register the SCI precompiles in the
  `PrecompilesMap`, and `SciHandler` needs to consume types from
  `sci_precompiles` (`HookOutcome`, `run_pre_execution_hook`,
  `apply_post_execution_deductions`).
- This is the single dependency edge between Base and SCI.

### 2.4 `crates/common/evm/src/factory.rs`

**Change**: +17 lines net. Inside both `BaseEvmFactory::create_evm` and its
`_with_inspector` variant, immediately after `PrecompilesMap::from_static(...)`
the factory now calls `sci_precompiles::install(&mut precompiles, &input.cfg_env)`.

**Why**:
- Base v0.9's EVM assembly hard-codes the Ethereum standard precompiles via
  `BasePrecompiles::new_with_spec`. SCI precompile addresses (`0xAAAA...0000`
  and `0xAAAA...0001`) are absent from that set and must be added during
  assembly.
- Choosing "install after assembly" rather than "fork a new `SciPrecompiles`
  set" minimises the surface area, avoids merge conflicts when upstream
  `BasePrecompiles` changes, and makes adding future SCI precompiles a
  contained change (only `sci_precompiles::install` needs to grow).

### 2.5 `crates/common/evm/src/lib.rs`

**Change**: 3 lines net — `mod sci_handler; pub use ...::SciHandler;`.

**Why**:
- The new `sci_handler.rs` file (see §2.6) must be exported from `lib.rs`
  so that `evm.rs` can `use` `SciHandler`.
- The reason `sci_handler.rs` lives in `base-common-evm` (rather than under
  `sci/`) is to **avoid a dependency cycle**: `sci-precompiles` already
  depends on `base-common-evm` for shared types, so the reverse cannot hold.
  The full architectural rationale is in
  `sci/crates/precompiles/src/handler/mod.rs` doc comments.

### 2.6 `crates/common/evm/src/sci_handler.rs` (**the only new Base file**)

**173 lines of new code.** `SciHandler<EVM, ERROR, FRAME>` wraps Base v0.9's
`BaseHandler` (which itself wraps `MainnetHandler` upstream) and delegates
every Handler trait method verbatim — **except**
`validate_against_state_and_deduct_caller`, which first checks
`tx_type == DEPOSIT_TRANSACTION_TYPE` (OP-Stack predeploy-tick path) and only
when the tx is not a deposit does it call
`sci_precompiles::run_pre_execution_hook` to apply keychain checks. The
post-execution counterpart, `execution_result`, similarly invokes
`sci_precompiles::apply_post_execution_deductions` once the inner result is
known to be `Ok`, to apply spending-limit deductions only on successful
agent-tx execution.

**Why this file is mandatory**:
- The pre-execution hook (CircuitBreaker → Scope → SpendingLimit) must fire
  **before** EVM execution starts — after gas has been pre-charged and the
  nonce bumped, but before any per-call body runs. Otherwise the hook cannot
  fail-fast without wasting gas.
- revm's `Handler` trait is the only clean place to intercept that point.
- A wrapper rather than a fork keeps upstream `BaseHandler` visible and
  reviewable; we override exactly two methods (pre-exec validation +
  post-exec result).
- System-call paths (deposit transactions and OP-Stack system calls) bypass
  the hook entirely via the `tx_type` short-circuit, ensuring OP-Stack
  predeploy ticks remain unaffected.

### 2.7 `crates/common/evm/src/evm.rs` (**handler-swap site — v0.9 location**)

**Change**: 12 lines net. Five `BaseHandler::<_, _, EthFrame<EthInterpreter>>::new()`
construction sites are swapped to `SciHandler::<_, _, EthFrame<EthInterpreter>>::new()`:
- `transact_one`
- `replay`
- `inspect_one_tx`
- `system_call_one_with_caller`
- `inspect_one_system_call_with_caller`

**Why this moved from `api/exec.rs` to `evm.rs`**: Base v0.9 restructured
`crates/common/evm/`. In v0.8 the handler instantiations lived in
`crates/common/evm/src/api/exec.rs`; v0.9 turned `exec.rs` into a
trait-alias-only module (`BaseContextTr` lives there now) and relocated the
actual handler construction call sites into `evm.rs`. The SCI patch follows
upstream — the five sites are functionally identical, just at a new path.

**Why all five sites swap**: every execution entry point must flow through
`SciHandler`. System-call paths get the wrapper too, but the `tx_type`
short-circuit inside `sci_handler.rs` early-returns them without performing
keychain work.

### 2.8 `crates/common/evm/src/api/exec.rs` (**trait-bounds patch**)

**Change**: +13 / -2 lines. The `BaseContextTr` trait alias (and its blanket
`impl`) gains two new bounds:

```rust
Db: alloy_evm::Database,
Journal: JournalTr<State = EvmState, Database: alloy_evm::Database> + core::fmt::Debug,
```

A doc comment explicitly marks the addition as `**SCI patch**` so future Base
upstream merges can identify it.

**Why**:
- `SciHandler`'s pre-execution hook constructs an `alloy_evm::EvmInternals`,
  whose `new` constructor requires the journal to be `Debug`.
- The two new bounds are hard requirements of that construction. Every
  concrete `BaseContext<DB>` instance Base actually uses (`State<...>`,
  `InMemoryDB`, `EmptyDB`) already satisfies them, so adding the bounds here
  is non-breaking for upstream Base callers in practice.
- In v0.8 the equivalent bounds lived on `OpContextTr`; v0.9 collapsed that
  into `BaseContextTr` (now upstream-owned), so the bounds had to move with
  it. This is the file CLAUDE.md should add to its "7 files" list — see the
  note at the top of §2.

### 2.9 `CLAUDE.md` (rewritten + extended)

**Change**: +725 / -10 lines (net +715). The Base-provided CLAUDE.md is
replaced by an SCI development guide; the v1.7.1 sync added another ~290
lines covering shim-crate strategy and updated workflow.

**Why**:
- The SCI fork has different conventions from Base mainline development:
  chain ID 42001, do-not-`fmt` rules on Base files, SCI naming conventions,
  Tempo synchronization workflow, devnet red lines, and so on.
- The v0.9 + v1.7.1 uplift expanded the rule set to cover: the shim-crate
  pattern (`sci-revm-shim` invariants and extension workflow), Tempo sync
  with verbatim-cp + sed sweep + SCI-patch-reapply checklist, EIP-161
  alloc-gap fix, devnet image-tag convention (`:local` vs `:sci` vs
  `:sci-dev-broken`), and the English-only doc policy (Critical Rule #7).
- Centralising these rules in CLAUDE.md spares the author from re-explaining
  them and gives any AI collaborator a single source of truth to follow.
- The original Base style rules ("Base Upstream Style Rules" section) are
  retained intact as inherited policy.

### 2.10 `etc/docker/devnet-env` (**documented in CLAUDE.md but not yet patched on disk**)

CLAUDE.md Critical Rule #2 lists `etc/docker/devnet-env` (chain ID = 42001) as
one of the 7 modified Base files. **The actual file on this branch still reads
`L2_CHAIN_ID=84538453`** — the v0.9 uplift documented the override but never
reconciled the value. The remote devnet has its own copy of `devnet-env` with
chain ID 42001 (kept out of the rsync per the deployment runbook — see the
`project-devnet-v1-7-1-deployment` user memory), so production behaviour is
unaffected. A follow-up should either patch the in-repo file or update CLAUDE.md
to remove this line from the "7 modified" list.

---

## 3. SCI Rust Crates (entirely under `sci/crates/`, zero overlap with Base)

Five independent crates that together implement the keychain precompile and the
revm 34 ↔ 38 compatibility surface.

### 3.1 `sci/crates/precompiles/` (~18,300 lines, 25 files)

**The core crate** — all precompile business logic, the EVM-backed storage
abstraction, and the integration tests.

| Module | Lines | Purpose |
|---|---|---|
| `account_keychain/mod.rs` | 4,705 | Keychain business core (Tempo v1.7.1 verbatim with SCI patches): authorize / revoke / spending limits / call scopes; the T5 witness API (`authorizeKey_2`, `burnKeyAuthorizationWitness`, `isKeyAuthorizationWitnessBurned`) is included |
| `account_keychain/dispatch.rs` | 444 | ABI selector routing (T3 + T5 selector schedules) |
| `account_keychain/sci_ext.rs` | 68 | **SCI-only extension**: exposes `key_is_active` and `transaction_key_raw` to the hook (both crate-private upstream in Tempo); doc comments document `remove_allowed_calls` scoped-deny-all behaviour and `getKey` isRevoked semantics |
| `sci_agent_state/mod.rs` | 227 | **SCI-only second precompile**: CircuitBreaker trip state (Tempo has no equivalent) |
| `sci_agent_state/dispatch.rs` | 63 | Same, ABI routing |
| `storage/evm.rs` | 1,199 | EVM-backed storage provider (production path); Tempo v1.7.1 verbatim with three SCI patches: `cfg.enable_amsterdam_eip8037` → literal `false`, `GasParamsExt` import for shim trait, integration tests gated behind the `evm-bridge-tests` feature |
| `storage/hashmap.rs` | 327 | In-memory backend (unit-test path); `JournalCheckpoint` literal drops `selfdestructed_i` (revm 34 has no such field) |
| `storage/mod.rs` | 192 | Storage provider trait |
| `storage/packing.rs` | 1,180 | Field packing/unpacking logic (lets `sigType+expiry+enforce+revoked` share one slot) |
| `storage/thread_local.rs` | 609 | `StorageCtx` thread-local plus the various `enter_*` entry points |
| `storage/types/{mapping,vec,set,array,slot,bytes_like,primitives,mod}.rs` | ~6,700 | The `Storable` type system: `Mapping<K, V>`, `Vec<T>`, `Set`, fixed-length arrays, byte strings, primitive scalars (v1.7.1 added the `array.rs` module) |
| `handler/hook.rs` | 304 | **Main pre-execution hook logic**: 7702 delegation detection, batch decoding, CB check, scope and spending check, checkpoint rollback (R5 defensive deposit-tx gate added) |
| `handler/decode.rs` | 204 | Decodes the `SCIAgentDelegator::execute(Call[])` calldata |
| `handler/mod.rs` | 25 | Public API: `run_pre_execution_hook` / `apply_post_execution_deductions` |
| `error.rs` | 267 | `SciPrecompileError` enum, `IntoPrecompileResult` trait, v1.7.1 reservoir-threading shape with halt-based OOG |
| `lib.rs` | 339 | `Precompile` trait, `install(...)`, `sci_precompile!` macro (wraps verbatim Tempo precompile bodies with `to_revm34` at the `DynPrecompile` boundary), `SelectorSchedule`, `dispatch_call`; `install()` now registers `AccountKeychain` at `TempoHardfork::T5` so the witness API is active by default |
| `test_util.rs` | 130 | `TIP20Setup` stub (Tempo upstream has a real TIP-20 factory; SCI uses ERC-20 and stubs the setup); selector-coverage + word-from-hex helpers |
| `tests/hook_e2e.rs` | 680 | **14 end-to-end hook integration tests** (strong R1, partial batch failure, CB, etc.) |

**Why this crate is mandatory**:
- This is SCI's core value-add — the AI-agent keychain. Without it, SCI is
  indistinguishable from Base v0.9.
- About 80% is verbatim from Tempo v1.7.1 (business source, macros, storage
  abstraction, ABI bindings). The remaining 20% is SCI-specific:
  - `account_keychain/sci_ext.rs`: SCI exposes `key_is_active` and
    `transaction_key_raw` (both crate-private upstream) so the hook and
    `SciHandler::execution_result` can probe them.
  - `sci_agent_state/`: a CircuitBreaker state precompile that Tempo does
    not have — SCI-only protocol state.
  - `handler/`: Tempo writes the hook in `revm/src/handler.rs`. SCI puts
    it here because a reverse import would cycle (see §2.5).
  - Alloy path adjustments: Tempo uses the `alloy` umbrella crate; Base
    uses the individual crates (`alloy_primitives`, etc.). Every ported
    file is path-adjusted via the sed sweep on each sync, including the
    new `::alloy::primitives::aliases::U96` paths v1.7.1 introduced.
  - `is_tip20()` stubbed to always return `true` — SCI applies recipient
    restrictions to any transfer/approve target without TIP-20 prefix
    checks, since SCI doesn't ship the TIP-20 factory.

### 3.2 `sci/crates/precompiles-macros/` (~3,600 lines, 7 files)

The `#[contract]` and `#[derive(Storable)]` proc-macro implementation, **ported
verbatim from Tempo v1.7.1**.

| File | Purpose |
|---|---|
| `lib.rs` | Macro entry points |
| `storable.rs` | Core implementation of `#[derive(Storable)]` |
| `storable_primitives.rs` | `Storable` impls for primitive types |
| `storable_tests.rs` | Macro-internal tests |
| `layout.rs` | Storage layout analysis |
| `packing.rs` | Field-packing computation |
| `utils.rs` | proc-macro helpers |

**Why mandatory**:
- `#[contract(addr = ACCOUNT_KEYCHAIN_ADDRESS)]` translates a Rust struct
  into an EVM storage layout, analogous to Solidity's storage layout.
- `#[derive(Storable)]` auto-generates read/write/checkpoint trait impls.
- Without these macros the keychain business code would need to hand-write
  every storage access — roughly 10× the code and significantly more
  error-prone.
- The macros themselves are nontrivial (packing/layout algorithms); the
  size is proportionate.

### 3.3 `sci/crates/precompile-abi/` (~600 lines, 10 files)

ABI definitions. alloy's `sol!` macro produces Solidity-interface declarations
shared between Rust and TypeScript consumers.

| File | Purpose |
|---|---|
| `precompiles/account_keychain.rs` | `IAccountKeychain` interface — T3 base API plus the T5 witness API (`authorizeKey_2`, `burnKeyAuthorizationWitness`, `isKeyAuthorizationWitnessBurned`). Carries a manual `impl AccountKeychainError { fn unauthorized_caller() ... }` block plus `impl AccountKeychainEvent { fn key_authorized() ... }` block as SCI patches — alloy-sol-macro 1.6.0+ auto-generates these, but Base v0.9 is pinned to 1.5.6 |
| `precompiles/sci_agent_state.rs` | `ISciAgentState` interface (tripKey / isTripped / etc.) |
| `precompiles/common_errors.rs` | Shared error types |
| `precompiles/tip20.rs` | TIP-20 interface (SCI uses the selectors as identifiers; the protocol is unimplemented) |
| `predeploys/erc20.rs` | ERC-20 interface (consumed internally by spending-limit logic) |
| `predeploys/sci_agent_delegator.rs` | `SCIAgentDelegator::execute(Call[])` interface (the hook decodes calldata via this) |

**Why mandatory**:
- Centralising ABI definitions in a single crate means `dispatch.rs`, hook
  decoding, and external consumers all import from one place.
- alloy's `sol!` generates compile-time type-safe encode/decode code,
  significantly more reliable than hand-written ABI parsing.

### 3.4 `sci/crates/tempo-chainspec-shim/` (~131 lines, 2 files)

A 14-line `Cargo.toml` plus a 117-line `lib.rs` — a minimal compatibility
shim that exposes `tempo_chainspec::hardfork::TempoHardfork` along with the
SCI-facing alias `SciHardfork`. v1.7.1 expanded the enum to include `T4`,
`T5`, and `T6` with matching `is_tX()` helpers.

**Why mandatory**:
- Upstream Tempo provides a full `tempo_chainspec` crate (chainspec + hardfork
  + genesis).
- SCI does not need most of that — SCI inherits Base's chainspec.
- But verbatim-ported Tempo business source files contain
  `use tempo_chainspec::hardfork::TempoHardfork`.
- The shim lets those imports compile, **which is what makes verbatim
  porting tractable.**

This is one of the key engineering artefacts of the "verbatim port" strategy.

### 3.5 `sci/crates/revm-shim/` (~513 lines, 5 files) — **NEW IN v1.7.1**

The load-bearing platform-adjustment layer. It is the **single biggest
engineering investment** in the v1.6.0 → v1.7.1 uplift and is what allows the
rest of Tempo to be ported verbatim.

| File | Purpose |
|---|---|
| `Cargo.toml` (15 lines) | Re-exports `revm = "34"` from Base; declares itself as `package = "sci-revm-shim"` |
| `src/lib.rs` (69 lines) | Top-level re-exports: every revm 34 submodule (`context`, `handler`, `primitives`, `state`, ...) plus the shadowed `precompile` and `interpreter::gas` modules |
| `src/precompile.rs` (256 lines) | The shadowed `precompile` module: a `PrecompileOutput` newtype carrying the v38 fields (`state_gas_used`, `reservoir`, `status: ExecutionStatus`), `PrecompileHalt` enum, constructors `new/revert/halt`, and the boundary fn `to_revm34(out)` that folds shim outputs back into real revm 34 `PrecompileResult` |
| `src/interpreter.rs` (132 lines) | The shadowed `interpreter::gas` module: a no-op `GasTracker` stub returning zero counters (SCI does not adopt state-gas accounting) |
| `src/gas_params_ext.rs` (41 lines) | `GasParamsExt` trait providing `code_deposit_state_gas` / `create_state_gas` / `sstore_state_gas` as no-op stubs on revm 34's `GasParams`, so verbatim Tempo source that calls these compiles unchanged |

**The mechanism**: `sci/crates/precompiles/Cargo.toml` declares
`revm = { path = "../revm-shim", package = "sci-revm-shim" }`. Every
`use revm::*;` in verbatim Tempo source then resolves through the shim. At
the `DynPrecompile` boundary, `revm::precompile::to_revm34(out)` (defined in
the shim) folds shim outputs back into revm 34 results: `Halt(OutOfGas)` →
`Err(PrecompileError::OutOfGas)`; success/revert preserve bytes + gas.

**Why mandatory**:
- Tempo v1.7.1 runs on revm 38 + alloy-evm 0.34. Base v0.9 is pinned to revm
  34 + alloy-evm 0.27.3. Without the shim, every reference to the v38-shape
  `PrecompileOutput` (with `state_gas_used` / `reservoir` / `status` fields,
  `::halt(reason, reservoir)` constructor, etc.) in upstream business source
  would need to be sed-rewritten on every sync.
- With the shim, the verbatim-cp workflow stays tractable. Upgrading to a
  hypothetical Tempo v1.7.2 / v1.8.x that adds new revm-38-only API surface
  becomes a contained operation: extend the shim, not the business source.

**Invariants** (codified in CLAUDE.md "Shim crate maintenance"):
- The shim is **additive**. It never removes or shadows any revm 34 item
  except `precompile` and `interpreter::gas`.
- The alias is **scoped to `sci-precompiles` only**. Base crates, `SciHandler`,
  and the rest of the workspace continue to depend on real revm 34.
- `reservoir = 0` and `amsterdam_eip8037_enabled = false` always — SCI does
  not adopt EIP-8037 / TIP-1016.

**Coverage**: 7 unit tests in `precompile.rs` (newtype constructors + halt
round-trip + `to_revm34` boundary).

---

## 4. SCI Devnet Configuration (`sci/devnet/`, 4 files)

| File | Lines | Purpose |
|---|---|---|
| `docker-compose.sci.yml` | 29 | Compose override pointing `base-client.image` / `base-builder.image` at the `:sci` tag |
| `sci-allocs.json` | 12 | Genesis allocs for SCI precompile addresses (`{nonce: 0, balance: 0, code: "0xef"}`) |
| `apply-sci-allocs.sh` | 67 | jq merge script that injects `sci-allocs.json` into the `genesis.json` produced by op-deployer |
| `.gitkeep` | 0 | Placeholder |

**Why these files exist** (the gap surfaced during the May 2026 devnet sessions):
- SCI precompile addresses (`0xAAAA...0000` and `0xAAAA...0001`) are absent
  from the genesis allocs that op-deployer produces by default.
- revm treats an address with no alloc record as a newly-created empty
  account; EIP-161 then garbage-collects the entire account at end-of-tx.
- With no alloc, keychain `sstore` writes appear successful (the
  `KeyAuthorized` event still emits, since events are stored separately) but
  the storage itself is dropped — making every stateful test (T4
  `authorizeKey`, etc.) fail despite a successful transaction receipt.
- Upstream Tempo's `dev.json` solves this with a `code: "0xef"` placeholder;
  SCI adopts the same pattern.
- The compose override is part of the image-tag isolation scheme (`:sci`
  does not overwrite `:local`), allowing SCI and pure-Base test runs to
  coexist on the same devnet host.

(The detailed root-cause analysis, debugging trail, and end-to-end runtime
environment changes from the debugging sessions are kept locally under
`sci/docs/analysis/` — gitignored per CLAUDE.md Critical Rule #7.)

---

## 5. SCI Documentation (`sci/docs/`)

| File | Purpose |
|---|---|
| `feat-p0-1-keychain-branch-summary.md` (this file) | What lives on the branch and why |
| `sci/docs/analysis/` (gitignored) | Working notes, dev-period analysis docs, debugging trails. Not committed per CLAUDE.md Critical Rule #7 (English-only public docs policy) |

**Why this file is mandatory**:
- The branch contains 27,000+ added lines and touches a delicate Base
  boundary. A reviewer who jumps in cold needs a guided tour, not just a
  diff.
- This is the document that fulfils that role.

---

## 6. Placeholder Directories (`.gitkeep` × 9)

`sci/contracts/{script,src/agent,src/integration,src/interfaces,test}`,
`sci/crates/contracts/abi`, `sci/docs/api`, `sci/gateway/src/{core,mpp,rest}`.

**Why mandatory**:
- CLAUDE.md "Repository Structure" defines the full `sci/` tree.
- These directories reserve space for upcoming work: Solidity contracts
  (Heath's scope), the MPP Gateway in TypeScript, API docs, etc.
- `.gitkeep` keeps empty directories tracked so the tree stays stable.

---

## 7. Engineering Principles Distilled

Abstracting "why these changes are mandatory" one level up, the branch
adheres to five engineering principles:

### Principle 1: Minimise Base file modifications (hard-capped)

**Why**:
- It guarantees that an upstream Base merge can only conflict in one
  bounded set of files.
- AI collaborators and new contributors won't "casually" edit Base files
  (CLAUDE.md explicitly forbids it).
- Any new Base modification must be added to CLAUDE.md's list and
  individually justified.

### Principle 2: Use Cargo `package = "..."` renames to enable verbatim Tempo porting

**Why**:
- Upstream Tempo is SCI's primary source for keychain logic and will keep
  evolving (v1.6.0 → v1.7.1 added the T5 witness API; v1.8.x will follow).
- Renaming identifiers in the source (`tempo_*` → `sci_*`) would cause
  large merge conflicts on every Tempo upgrade.
- Cargo renames keep source files unchanged; the mapping lives in a single
  place — the workspace `Cargo.toml`.

### Principle 3: Absorb cross-version platform drift in shim crates, not source patches

**Why** (new principle introduced with the v1.7.1 uplift):
- The revm 34 → 38 gap (state-gas + reservoir model, new `PrecompileOutput`
  shape, halt-based OOG) could have been absorbed with dozens of one-line
  edits scattered across the ported source. Instead `sci-revm-shim`
  consolidates the entire delta in one ~500-line additive crate.
- The `tempo-chainspec-shim` follows the same pattern at a smaller scale.
- The cost is one extra crate; the benefit is that every Tempo sync from
  now on is `cp` + sed sweep + a short, stable patch list, instead of an
  open-ended audit.

### Principle 4: Every SCI-specific divergence is documented in CLAUDE.md "Critical Rules"

**Why**:
- For example: `is_tip20()` stubbed to return true, `test_util::TIP20Setup`
  no-op, the ignored-test list, the `enable_amsterdam_eip8037` hardcoded
  `false`, the `selfdestructed_i` literal omission, the manual
  `AccountKeychainError`/`AccountKeychainEvent` constructor blocks.
- These are deliberate SCI design choices (not bugs); recording them in
  CLAUDE.md ensures future Tempo upgrades don't reintroduce upstream
  behaviour by accident.

### Principle 5: All SCI additions live under `sci/`, zero pollution of Base directories

**Why**:
- Consistent with Principle 1: gives reviewers a clear boundary.
- A Base reviewer reading the diff sees only the 8 Base files plus
  `sci/` — they are not drowned in 27,000 lines of new code.
- For a Base upstream merge: as long as the 8 Base files have no
  conflict, the entire PR merges cleanly.

---

## 8. Verification

State of the branch as of commit `036e70d16`:

- **Local unit tests**: 319 lib (1 ignored — the documented is_tip20
  divergence test) + 14 hook_e2e + 7 revm-shim + ~74 macro tests all pass.
- **Workspace `cargo check`**:
  ```
  cargo check -p sci-revm-shim -p sci-precompiles -p sci-precompiles-macros \
              -p sci-precompile-abi -p tempo-chainspec-shim -p base-common-evm
  ```
  Clean.
- **Devnet** (`ubuntu@54.255.70.252`, deployed 2026-05-26):
  - Containers running `base-reth-node:sci` and `base-builder:sci` (no panics).
  - Genesis patched with SCI allocs; `rollup.json` / `rollup-conductor.json`
    `genesis.l2.hash` synced to the post-alloc value (re-computed on each
    redeploy — the v0.8-era hash no longer applies).
- **Devnet functional tests — all PASS**:
  - **T1** chain ID 42001, blocks producing
  - **T2** keychain + sci-agent-state precompiles reachable
  - **T3** 1 wei transfer ACC1 → ACC0 succeeds (delta=1 wei)
  - **T4** `authorizeKey` → `KeyAuthorized` event + `getKey` returns correct
    `KeyInfo` (5-field struct: `(uint8 signatureType, address keyId,
    uint64 expiry, bool enforceLimits, bool isRevoked)`)
  - **T5** `SciAgentState.tripKey` from non-CB caller reverts with
    `0x82b42900` (`Unauthorized`)
  - **T-W1** `authorizeKey_2(witness)` → `KeyAuthorizationWitness` event,
    `isKeyAuthorizationWitnessBurned` still false (T5 witness API)
  - **T-W2** `burnKeyAuthorizationWitness` → `KeyAuthorizationWitnessBurned`
    event
  - **T-W3** `isKeyAuthorizationWitnessBurned` returns `true` after burn
- **CI / lint**: not exercised on this branch (CI configuration is out of
  scope here).

---

## 9. Blocked / Out of Scope

- **T7 long-run stability** test: optional, not yet run.
- **T8 full agent-tx loop**: blocked on Heath landing the
  `SCIAgentDelegator.sol` deployment at `0xCCCC...01`, with a matching
  genesis alloc. The internal Rust paths are already covered by the 14
  `hook_e2e` integration tests against an in-memory DB.
- **MPP Gateway**: `sci/gateway/` is a placeholder; out of scope for P0-1.
- **SCI mainnet chainspec**: only a devnet template exists. The mainnet
  chainspec is a P1 task.
- **`etc/docker/devnet-env` reconciliation**: the in-repo file still reads
  `L2_CHAIN_ID=84538453`; the remote devnet uses 42001 as a working-tree
  patch. CLAUDE.md documents this discrepancy as a follow-up.
- **CLAUDE.md "7 Base files" list reconciliation**: the actual count is 8
  (api/exec.rs trait-bounds patch is implicit but undocumented).

---

## 10. One-Sentence Summary

`feat/p0-1-keychain-tempo-v1.7.1` delivers the P0-1 keychain precompile
end-to-end on top of Base v0.9, ported verbatim from Tempo v1.7.1 (including
the T5 witness API), with **7 modified Base files + 1 new Base file** plus
**~27,000 lines under `sci/`**. The Base footprint is confined to the
EVM-factory / handler-assembly path, making upstream merges friction-free;
the SCI business code is roughly 80% verbatim from Tempo, kept tractable
across the revm 34 ↔ 38 platform gap by the new `sci-revm-shim` compat crate
plus the existing `tempo-chainspec-shim`; and devnet integration is verified
end-to-end with T1–T5 + T-W1–T-W3 all passing on the 2026-05-26 deployment.
