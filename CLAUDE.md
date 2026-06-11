# CLAUDE.md — SCI Chain Development Guide

## Project

SCI Chain is an Agent-native Ethereum L2, forked from Base Azul v0.9 (`base/base`).
It adds a protocol-level permission sandbox for AI Agents via the Keychain Precompile
(ported from Tempo v1.6.0), with MPP (Machine Payments Protocol) as the Agent access layer.

**Agent-tx mechanism — Plan A (native AA transaction type).** The agent's "act as root"
calls ride a native account-abstraction transaction type (`type 0x76`,
[`BaseAaTransaction`]) carrying a batch of `calls[]` and an optional `fee_payer`
(sponsored gas). A Rust pre-execution hook in the EVM handler decodes the batch and
applies the keychain checks (CircuitBreaker → Scope → SpendingLimit) before execution.
This supersedes — and fully replaces — the earlier **Plan B** design (standard EIP-1559 tx
+ EIP-7702 delegation to a `SCIAgentDelegator` predeploy + precompile hook). Plan B has been
removed from this branch: no `SCIAgentDelegator` / `SciAgentRegistrar` contracts, no `0xCCCC01`
predeploy, and no 7702 `run_pre_execution_hook` path — only the AA-native path remains.
Plan A work lives on branch `feat/plan-a-aa-keychain`; see `sci/docs/test/plan-a-status.md`
for the phase tracker. The Keychain Precompile, shim crates, and Tempo-sync workflow are
unchanged by the Plan A pivot — only the agent-tx carrier and the set of touched Base
files differ.

Chain ID: 42001 | Rust edition: 2024 | Rust version: 1.93.1 | Linker: mold

## Architecture

```
Agent → mppx.fetch() → SCI Agent Gateway (MPP 402 + REST)
                              ↓ JSON-RPC (AA tx, type 0x76)
                        SCI Chain (Base Azul v0.9 fork)
                          AA tx type 0x76 (BaseAaTransaction): calls[] + fee_payer
                          Pre-execution hook: CircuitBreaker → Scope → SpendingLimit
                          Precompile: 0xAAAA.. AccountKeychain
                          Predeploys: 0xBBBB01 Registry, 0xBBBB02 Budget, 0xBBBB03 Breaker
```

## Repository Structure

```
sci-chain/
├── crates/                    ← Base original Rust code (DO NOT add files here)
│   ├── common/evm/            ← Base crate we modify (precompile + SciHandler hook)
│   ├── common/consensus/      ← Plan A: AA tx type 0x76 (aa.rs + envelope/pooled/codec)
│   ├── common/rpc-types/      ← Plan A: AA → TransactionRequest arm
│   └── execution/{evm,flashblocks,txpool}/ ← Plan A: AA receipt/pool/validator arms
├── etc/docker/devnet-env      ← Modified: Chain ID 42001
├── sci/                       ← ALL SCI additions go here
│   ├── crates/                ←   Rust (Keychain precompile)
│   │   ├── precompiles/       ←     Core: AccountKeychain, storage abstraction
│   │   ├── precompiles-macros/←     Proc macros (#[contract], #[Storable])
│   │   ├── precompile-abi/    ←     Precompile ABI bindings (alloy sol!)
│   │   ├── revm-shim/         ←     Compat shim exposing revm 38 PrecompileOutput /
│   │   │                             PrecompileHalt / state-gas API surface on top
│   │   │                             of Base v0.9's revm 34 (sci-precompiles-only)
│   │   └── tempo-chainspec-shim/ ← Compat shim exposing `tempo_chainspec::hardfork`
│   │                                so verbatim Tempo source compiles unmodified
│   ├── contracts/             ←   Solidity (Foundry project)
│   │   ├── src/agent/         ←     P0-2: AccessKeyRegistry, BudgetController, CircuitBreaker
│   │   └── src/interfaces/    ←     Public interfaces (other repos depend on these)
│   ├── gateway/               ←   TypeScript (MPP Server + REST API)
│   ├── devnet/                ←   Genesis patch + allocs
│   └── docs/                  ←   Project documentation
└── Cargo.toml                 ← Modified: workspace members include sci/crates/*
```

## Critical Rules

1. **Prefer adding files under `sci/`** (`crates/`, `bin/`, `devnet/`, `etc/`, `docs/`,
   `actions/`, `baseup/` are Base directories). The keychain-precompile integration adds
   exactly **one** new Base file (`sci_handler.rs`); everything else for it lives under
   `sci/`. **Plan A is the exception**: a native transaction type cannot be expressed as
   an `sci/`-only addition — it must be threaded through Base's shared `BaseTxEnvelope`
   enum and its consumers (consensus codec, rpc-types, execution receipt/pool paths), so
   Plan A modifies a broader set of Base files in place. See Rule #2 group B.
2. **Touched Base files are tracked in two groups.** Any new Base modification must be
   added to the relevant group here and justified.

   **Group A — Keychain precompile integration** (6 modified, 1 added; kept intentionally
   small):
   - `Cargo.toml` — workspace members include `sci/crates/*` and corresponding
     `workspace.dependencies` entries (`sci-precompiles`, `sci-precompiles-macros`,
     `sci-precompile-abi`, `sci-revm-shim`).
   - `crates/common/evm/Cargo.toml` — adds `sci-precompiles.workspace = true` so the
     factory can install the AccountKeychain precompile.
   - `crates/common/evm/src/factory.rs` — calls `sci_precompiles::install(&mut precompiles, &cfg_env)`
     immediately after `PrecompilesMap::from_static(...)` in both `create_evm` and
     `create_evm_with_inspector`.
   - `crates/common/evm/src/lib.rs` — declares `mod sci_handler; pub use ...::SciHandler;`
     next to Base v0.9's own `mod handler; pub use handler::{BaseHandler, IsTxError};`
     so `evm.rs` (and external consumers) can import the wrapper handler.
   - `crates/common/evm/src/sci_handler.rs` (**new file**) — `SciHandler<EVM, ERROR, FRAME>`
     wraps `BaseHandler` (Base v0.9's own handler, which itself wraps `MainnetHandler`),
     delegates every Handler trait method verbatim except
     `validate_against_state_and_deduct_caller`, which (after deposit-tx short-circuit)
     calls `sci_precompiles::run_pre_execution_hook` to apply keychain checks. The
     `Db: alloy_evm::Database` and `Journal: ... + Debug` trait bounds needed by the
     hook's `EvmInternals` construction live on this file's `Handler`/`InspectorHandler`
     impl blocks (they used to be on `OpContextTr` in v0.8 but `BaseContextTr` in v0.9
     is now upstream and left untouched). Lives in Base because `sci-precompiles` already
     depends on `base-common-evm` and the reverse would cycle; see
     `sci/crates/precompiles/src/handler/mod.rs` for the architecture rationale.
   - `crates/common/evm/src/evm.rs` — five `BaseHandler::<_, _, EthFrame<EthInterpreter>>::new()`
     construction sites swapped to `SciHandler::<_, _, EthFrame<EthInterpreter>>::new()`:
     `transact_one`, `replay`, `inspect_one_tx`, `system_call_one_with_caller`,
     `inspect_one_system_call_with_caller`. (In Base v0.8 these lived in
     `crates/common/evm/src/api/exec.rs`; v0.9 restructured `exec.rs` into a
     trait-alias-only module and moved the handler instantiations here.) System-call
     paths still get `SciHandler`, but its `validate_against_state_and_deduct_caller`
     short-circuits the `0x76` agent keychain hook on `tx_type == DEPOSIT_TRANSACTION_TYPE`
     so OP-Stack predeploy ticks bypass the per-call scope/limit/CB gate. (As of the L1
     escape-hatch Tier 2 work it does **not** fully early-return for deposits: it still seeds
     the keychain transient `tx_origin = tx.caller` via `set_keychain_tx_origin` so a
     force-included keychain admin call from L1 passes `ensure_account_caller` — see
     `sci/docs/plan-a-l1-escape-hatch.md` §5. The seed is transient (TSTORE), so it does not
     perturb system-deposit state roots.)
   - `etc/docker/devnet-env` — Chain ID 42001 (note: as of the v0.9 uplift this
     override is documented but not yet applied to the file — the line still reads
     `L2_CHAIN_ID=84538453`; a follow-up should reconcile docs vs. file).

   **Group B — Plan A AA transaction type** (`type 0x76`; branch `feat/plan-a-aa-keychain`;
   see `sci/docs/test/plan-a-status.md`). A new tx type must be threaded through Base's
   shared envelope and every match site that enumerates tx variants, so these Base files
   are modified in place (unavoidable for a native tx type):
   - `crates/common/consensus/src/transaction/aa.rs` (**new file**) — `BaseAaTransaction`
     (chain_id / nonce / 1559 fees / `calls[]` / access_list / `fee_payer`) + `Call`,
     RLP/2718 codec, `Transaction`/`Typed2718`/`SignableTransaction` impls, and the PoC
     `to_eip1559_first_call()` approximation helper.
   - `crates/common/consensus/src/transaction/{envelope,typed,tx_type}.rs` — `Aa` variant
     wired into `BaseTxEnvelope` / `BaseTypedTransaction` / `OpTxType` (~40 match arms);
     `try_into_pooled` accepts AA, `try_into_eth_pooled` rejects it (no alloy repr).
   - `crates/common/consensus/src/transaction/pooled.rs` — `Aa` in `BasePooledTransaction`
     (**local-only**: see Plan A divergences); alloy-only conversions are documented
     `unreachable!`.
   - `crates/common/consensus/src/transaction/core.rs` — `FromTxWithEncoded` execution arm.
   - `crates/common/consensus/src/reth_compat.rs` — reth `Compact` (`CompactCall` /
     `CompactBaseAaTransaction` mirror `CompactTxDeposit`), `OpTxType` extended-id arm,
     `ToTxCompact`/`FromTxCompact`, `InMemorySize`; receipt mapping `OpTxType::Aa → Eip1559`.
   - `crates/common/consensus/src/{lib,transaction/mod}.rs` — re-exports
     (`BaseAaTransaction`, `Call`, `SCI_AA_TX_TYPE_ID`, `CompactCall`,
     `CompactBaseAaTransaction`) + bincode-compat `Aa` variant (`Cow<'a, BaseAaTransaction>`).
   - `crates/common/rpc-types/src/transaction/request.rs` — AA → `TransactionRequest`
     (first-call EIP-1559 PoC approximation).
   - `crates/execution/evm/src/receipts.rs`,
     `crates/execution/flashblocks/src/receipt_builder.rs` — AA receipt arm (EIP-1559).
   - `crates/execution/txpool/src/validator.rs` (+ `Cargo.toml` adds
     `sci-precompiles.workspace = true`) — AA local-only: force `propagate = false`
     (no gossip). Admission is NOT origin-gated — a `--txpool.nolocals` sequencer tags RPC
     txs as `External`, so an origin reject would block legitimate RPC ingress (learned on
     devnet 2026-06-02). Admission IS keychain-gated for **sponsored** AA txs
     (`check_aa_keychain_authorization`, 2026-06-10 review finding M-3): `fee_payer` must
     equal `root` (structural — the handler rejects any other shape), and the signer must
     have a plausibly-active `keys[root][signer]` record (raw slot read via
     `AccountKeychain::authorized_key_slot` + `authorized_key_word_is_active` from
     `sci_ext.rs`), else fresh zero-balance signers naming a funded victim as fee_payer
     could stuff the pool at zero cost. Advisory only — the execution hook stays
     authoritative; fails open on state-provider errors.
   - `crates/execution/node/src/node.rs` — register AA tx type 0x76 via
     `EthTransactionValidatorBuilder::with_custom_tx_type` so reth's inner validator does
     not reject it as `TxTypeNotSupported`.
3. **Tempo code is reference only**. Source is at `/home/gavin/opensci/sci-dev/tempo/`
   (an earlier draft of this guide listed `~/sci-dev/Tempo-ref/` — that path does not exist
   on this machine). Copy and adapt, never import as a git dependency.
4. **Namespace convention — verbatim Tempo source, SCI-facing API via aliases + shim crates.**
   To keep upstream Tempo merges tractable, **ported Tempo source files use Tempo names
   internally** (`tempo_chainspec::hardfork::TempoHardfork`, `tempo_contracts::*`,
   `tempo_precompiles_macros::*`, `TempoPrecompileError`). Those names route to our
   `sci-*` crates via Cargo `package = ...` renames in the workspace `Cargo.toml`,
   and our `sci-precompiles` crate re-exports them as `SciHardfork`,
   `SciPrecompileError` (etc.) for SCI-facing consumers. Both names refer to the same
   type. Map:
   - `tempo_precompiles` (concept) → crate `sci-precompiles` (no source rename; Tempo
     doesn't ship a `tempo_precompiles` crate-level import that we re-host)
   - `tempo_precompiles_macros` → cargo-renamed to `sci-precompiles-macros`
   - `tempo_contracts` → cargo-renamed to `sci-precompile-abi`
   - `tempo_chainspec` → cargo-renamed to `tempo-chainspec-shim` (a ~100-line compat
     crate that exposes only `hardfork::TempoHardfork` + `SciHardfork` alias, with
     enum variants up through T6)
   - **`revm` (inside `sci-precompiles` only) → cargo-renamed to `sci-revm-shim`** (a
     ~250-line compat crate that re-exports real revm 34 verbatim and ADDS the
     v38-shape `PrecompileOutput` newtype with `state_gas_used` / `reservoir` /
     `status` fields + `PrecompileHalt` enum + `GasParamsExt` trait + `GasTracker`
     stub + `to_revm34` boundary fn. Verbatim Tempo v1.7.1+ source that imports
     `revm::precompile::PrecompileHalt` or constructs
     `PrecompileOutput::halt(reason, reservoir)` compiles unmodified through the
     shim, and SCI's `install()` macro folds shim outputs back into revm 34's
     `PrecompileResult` via `revm::precompile::to_revm34(...)`. The alias is
     **scoped strictly to sci-precompiles** — every other workspace member
     (including `base-common-evm` and our `SciHandler` host integration) continues
     to depend on real revm 34 directly. See "Shim crate maintenance" below for
     the upgrade workflow.)
   - `TIP-20` → standard ERC-20 (no rename — SCI just doesn't ship a TIP-20 factory;
     the keychain treats every contract called via transfer/approve as token-like;
     see Critical Rule #5 below)
5. **SCI-specific divergences** baked into the port (these are the *real* deltas vs.
   Tempo; everything else syncs verbatim):
   - `is_tip20(target)` is stubbed to always return `true` (see `validate_selector_rules`
     in `account_keychain/mod.rs`) — SCI applies recipient restrictions to any
     transfer/approve target without checking for a TIP-20 prefix. The upstream check
     also calls into `tempo_primitives::TempoAddressExt` and `tip20_factory::TIP20Factory`,
     neither of which SCI ships, so those imports are dropped at sync time.
   - `test_util::TIP20Setup` is a no-op stub (lives in `sci-precompiles/src/test_util.rs`)
     so ported tests using it compile but the setup runs no real TIP-20 deploy logic.
   - `test_t3_rejects_recipient_constrained_scope_for_undeployed_tip20` is `#[ignore]`'d
     (the assertion contradicts the relaxed `is_tip20→true` rule).
   - The keychain's call_scope path checks `is_constrained_tip20_selector` using
     standard ERC-20 `transfer`/`approve` selectors (identical hashes as Tempo's
     ITIP20), so the gating effectively becomes "selector matches ERC-20 transfer-like".
   - `AccountKeychainError::*` and `AccountKeychainEvent::*` snake_case constructor
     helpers (`unauthorized_caller()`, `key_already_exists()`, `key_authorized()`,
     etc.) are manually re-added in `sci/crates/precompile-abi/src/precompiles/account_keychain.rs`.
     Upstream Tempo dropped these manual impls in v1.7.1 because alloy-sol-macro 1.6.0
     auto-generates them; Base v0.9 is pinned to alloy-sol-macro 1.5.6 which does NOT
     auto-generate, so SCI keeps the manual block. Re-apply on every Tempo sync; can
     be deleted once Base bumps alloy-sol-macro past 1.6.0.
   - `storage/evm.rs` reads `cfg.enable_amsterdam_eip8037` in three sites — patched
     to literal `false` because SCI's revm 34 `CfgEnv` has no such field; SCI does
     not adopt EIP-8037 / TIP-1016 state-gas accounting.
   - `storage/evm.rs` imports `revm::GasParamsExt` (shim trait) as a one-line SCI
     patch so the verbatim Tempo source's `gas_params.code_deposit_state_gas(...)` /
     `create_state_gas()` / `sstore_state_gas(...)` calls resolve to the shim's no-op
     stubs.
   - `storage/evm.rs` `#[cfg(test)] mod tests` is changed to
     `#[cfg(all(test, feature = "evm-bridge-tests"))]` because the upstream test
     fixtures pull in `tempo_evm::TempoEvmFactory` / `tempo_revm::*` which SCI does
     not ship. Keychain coverage runs via `HashMapStorageProvider` instead.
   - `storage/hashmap.rs` `JournalCheckpoint` literal drops the `selfdestructed_i: 0`
     field — revm 34's `JournalCheckpoint` has no such field.
   - `account_keychain/mod.rs::unrestricted_restrictions()` (test helper) is patched
     to build `KeyRestrictions` inline instead of routing through
     `tempo_alloy::provider::keychain::KeyRestrictions::default()`.
6. **Version divergence vs. Tempo** (absorbed by the shim crate):
   - Tempo v1.7.1 uses `revm 38` + `alloy-evm 0.34` + `alloy` umbrella crate 2.0.5
   - Base v0.9 uses `revm 34` + `alloy-evm 0.27.3` + individual `alloy-*` crates 1.8/1.5
   - The gap (revm 34 → 38 introduced EIP-8037 / TIP-1016 "state gas + reservoir"
     model, new `PrecompileOutput` fields, `::halt(...)` constructor, etc.) is bridged
     by **`sci/crates/revm-shim`** — see Critical Rule #4 and "Shim crate maintenance"
     below. With the shim in place, SCI does NOT have to track revm bumps until Base
     itself bumps revm.
   - `alloy` umbrella → individual `alloy-*` crates: sed at sync time.
     `::alloy::primitives::aliases::U96` etc. were added in v1.7.1 — the sed sweep
     handles this with the `aliases::` rule documented below.
7. **Docs committed to the repository must be written in English.** This applies to
   every `.md` under version control (CLAUDE.md, READMEs, `sci/docs/**` except
   `analysis/`, design notes, PR descriptions, commit messages). Inline code
   comments and identifiers follow the same rule. Transient working notes,
   intermediate analyses, and personal scratchpads belong in `sci/docs/analysis/`
   which is **gitignored** — use any language there. The purpose of the rule is
   to keep the public-facing repo legible to non-Chinese-speaking contributors,
   while still allowing fast Chinese-language iteration during development.

## Upstream Tempo Sync

SCI Chain forks Tempo at v1.7.1 and tracks upstream keychain improvements without
per-merge identifier rewrites. The combination of Cargo `package = ...` renames
(Rule #4) and the `sci-revm-shim` compat crate means business source files
(`account_keychain/{mod,dispatch}.rs`, `storage/*.rs`, `error.rs`, the macros, the
ABI bindings) can be **copied verbatim** from a newer Tempo release. The shim
absorbs the revm-version gap; only a small bundle of well-known SCI patches needs
re-applying.

### Workflow when Tempo releases v1.7.2 (or any upgrade)

```bash
TEMPO=/home/gavin/opensci/sci-dev/tempo

# 1. Preserve SCI-only sibling files that live next to verbatim ones.
cp sci/crates/precompiles/src/account_keychain/sci_ext.rs /tmp/sci_ext.rs.preserve

# 2. Sync business files (zero substitution thanks to Cargo renames + shim)
cp $TEMPO/crates/precompiles/src/account_keychain/mod.rs      sci/crates/precompiles/src/account_keychain/
cp $TEMPO/crates/precompiles/src/account_keychain/dispatch.rs sci/crates/precompiles/src/account_keychain/
cp $TEMPO/crates/precompiles/src/storage/evm.rs               sci/crates/precompiles/src/storage/
cp $TEMPO/crates/precompiles/src/storage/hashmap.rs           sci/crates/precompiles/src/storage/
cp $TEMPO/crates/precompiles/src/storage/mod.rs               sci/crates/precompiles/src/storage/
cp $TEMPO/crates/precompiles/src/storage/packing.rs           sci/crates/precompiles/src/storage/
cp $TEMPO/crates/precompiles/src/storage/thread_local.rs      sci/crates/precompiles/src/storage/
cp $TEMPO/crates/precompiles/src/storage/types/*.rs           sci/crates/precompiles/src/storage/types/
cp $TEMPO/crates/precompiles-macros/src/*.rs                  sci/crates/precompiles-macros/src/
cp $TEMPO/crates/contracts/src/precompiles/account_keychain.rs sci/crates/precompile-abi/src/precompiles/
cp $TEMPO/crates/contracts/src/precompiles/common_errors.rs    sci/crates/precompile-abi/src/precompiles/

# 3. Restore the SCI-only sibling file
cp /tmp/sci_ext.rs.preserve sci/crates/precompiles/src/account_keychain/sci_ext.rs

# 4. Apply the alloy umbrella → individual-crate sed sweep
find sci/crates/precompiles sci/crates/precompiles-macros sci/crates/precompile-abi \
  -name "*.rs" -exec sed -i \
  -e 's|::alloy::primitives::aliases::|::alloy_primitives::aliases::|g' \
  -e 's|::alloy::primitives::|::alloy_primitives::|g' \
  -e 's|::alloy::sol_types::|::alloy_sol_types::|g' \
  -e 's|::alloy::consensus::|::alloy_consensus::|g' \
  -e 's|use alloy::primitives|use alloy_primitives|g' \
  -e 's|use alloy::sol_types|use alloy_sol_types|g' \
  -e 's|use alloy::consensus|use alloy_consensus|g' \
  -e 's|alloy::primitives|alloy_primitives|g' \
  -e 's|alloy::sol_types|alloy_sol_types|g' \
  -e 's|alloy::consensus|alloy_consensus|g' \
  {} +

# Note: There are no `PrecompileOutput::*` constructor rewrites in this sed sweep.
# The shim crate (`sci-revm-shim`) provides `PrecompileOutput::new(g,b,r)`,
# `revert(g,b,r)`, `halt(h,r)` natively, so verbatim Tempo source compiles as-is.
# Same for `.is_revert()` (shim method) vs `.reverted` (revm 34 field) — the shim's
# newtype provides `.is_revert()`.

# 5. Re-apply the SCI patches enumerated in Critical Rule #5 (one-shot text edits;
# stable across syncs unless upstream restructures the surrounding code):
#   - `account_keychain/mod.rs` line 9: add `mod sci_ext;` after `pub mod dispatch;`
#   - `account_keychain/mod.rs` line 33: drop `use tempo_primitives::TempoAddressExt;`
#   - `account_keychain/mod.rs` line 39: drop `tip20_factory::TIP20Factory` from
#     the `use crate::{...}` block
#   - `account_keychain/mod.rs::validate_selector_rules`: replace the
#     `cached_is_tip20`/`TIP20Factory::new().is_tip20()`/`target.is_tip20()` closure
#     body with `Ok(true)` and rename `target` → `_target` in the fn signature
#   - `account_keychain/mod.rs::test_t3_rejects_recipient_constrained_scope_for_undeployed_tip20`:
#     prepend `#[ignore = "..."]` to the `#[test]` line
#   - `account_keychain/mod.rs::unrestricted_restrictions`: replace the
#     `tempo_alloy::provider::keychain::KeyRestrictions::default().into()` body
#     with an inline `KeyRestrictions { expiry: u64::MAX, .. }` literal
#   - `account_keychain/dispatch.rs`: convert the two multi-line
#     `use alloy::{ primitives::*, sol_types::* };` blocks into separate
#     `use alloy_primitives::*` / `use alloy_sol_types::*` imports
#   - `storage/evm.rs`: add `GasParamsExt` to the `revm::{...}` import block
#   - `storage/evm.rs`: replace `cfg.enable_amsterdam_eip8037` with `false` in
#     `new_max_gas` and `new_with_gas_limit`
#   - `storage/evm.rs`: change `#[cfg(test)] mod tests` to
#     `#[cfg(all(test, feature = "evm-bridge-tests"))]`
#   - `storage/evm.rs`: convert the multi-line `use alloy::{...}` block at top
#   - `storage/hashmap.rs`: remove the `selfdestructed_i: 0,` line from the
#     `JournalCheckpoint` literal
#   - `storage/thread_local.rs`: convert the multi-line `use alloy::{...}` block at top
#   - `test_util.rs`: keep our stubbed 130-line version (`git checkout HEAD --`
#     before running tests — DO NOT cp upstream's 470-line version)
#   - `precompile-abi/src/precompiles/account_keychain.rs`: re-append the
#     `impl AccountKeychainError { fn unauthorized_caller(), ... }` and
#     `impl AccountKeychainEvent { fn key_authorized(), ... }` constructor blocks
#     (alloy-sol-macro 1.5.6 doesn't auto-generate them; 1.6.0+ does)
#   - `precompile-abi/src/precompiles/mod.rs`: add `authorizeKeyWithWitnessCall`
#     to the `pub use account_keychain::{...}` list when new aliases land
#   - `error.rs`: hand-merge any new variants into the trimmed SCI enum (we keep
#     only `AccountKeychainError`, `SciAgentStateError`, `OutOfGas`, `Panic`,
#     `UnknownFunctionSelector`, `Fatal`)

# 6. Verify
cargo check -p sci-revm-shim -p sci-precompiles -p sci-precompiles-macros \
            -p sci-precompile-abi -p tempo-chainspec-shim
cargo test  -p sci-precompiles --lib            # 319+ unit tests + 1 ignored
cargo test  -p sci-precompiles --test hook_e2e  # 13 AA-native integration tests
```

What still requires human review on merge:
- New `TempoHardfork` variants — add to `tempo-chainspec-shim/src/lib.rs` with
  matching `is_tX()` helpers.
- New error variants added to `TempoPrecompileError` upstream — reconcile against our
  trimmed enum in `error.rs`.
- New ABI methods / errors / events added to `IAccountKeychain` — propagate to the
  `pub use account_keychain::{...}` list in `precompile-abi/src/precompiles/mod.rs`,
  and add matching snake_case constructors to the SCI `impl AccountKeychainError`/
  `impl AccountKeychainEvent` blocks.
- Any business logic that depends on TIP-20 factory state — needs an SCI-specific
  reconciliation (currently the only known site is `validate_selector_rules`).
- If upstream adds a new revm 38+ API surface that the shim doesn't yet expose
  (e.g., a new `PrecompileOutput` method or a new field reference on `GasParams`):
  extend `sci/crates/revm-shim/` rather than patching the verbatim source. See
  "Shim crate maintenance" below.

## Shim crate maintenance

`sci/crates/revm-shim` is the load-bearing platform-adjustment layer. It does
three jobs:

1. **Re-export revm 34 verbatim** for every submodule Tempo verbatim source
   touches (`context`, `handler`, `primitives`, `state`, ...).
2. **Shadow `precompile` and `interpreter::gas`** with shim modules that expose
   the v38-shape API on top of revm 34's actual data structures. The shadowed
   `precompile::PrecompileOutput` is a fresh newtype carrying the v38 fields
   (`state_gas_used`, `reservoir`, `status: ExecutionStatus`) plus the
   `new/revert/halt` constructors that all accept a trailing `reservoir` arg.
   The shadowed `interpreter::gas::GasTracker` is a no-op stub returning zero
   counters.
3. **Provide a boundary fn** `revm::precompile::to_revm34(out)` that the SCI
   `install()` macro calls right before yielding a `DynPrecompile`. It folds
   `Halt(OutOfGas)` → `Err(PrecompileError::OutOfGas)`, preserves bytes/gas for
   success/revert, and yields a real revm 34 `PrecompileResult`.

### Invariants

- **The shim is additive.** It never removes or shadows any revm 34 item except
  the two explicitly listed above. Adding new v38 surface here does not perturb
  anything downstream that still binds to real `revm::precompile::PrecompileOutput`.
- **The alias is scoped.** Only `sci/crates/precompiles/Cargo.toml` carries
  `revm = { path = "../revm-shim", package = "sci-revm-shim" }`. Base crates,
  `SciHandler`, and the rest of the workspace continue to depend on real revm 34.
- **`reservoir = 0` and `amsterdam_eip8037_enabled = false` always.** SCI does
  not adopt EIP-8037 / TIP-1016. Should we ever want to adopt state-gas
  accounting, replacing the no-op stubs with real semantics is the place to start.

### When upstream Tempo adds a new revm-38-only API call

Three options in order of preference:

1. **Extend the shim.** If the upstream call resolves to a missing item under
   `revm::*`, add the stub to `sci-revm-shim` (e.g., a new method on
   `PrecompileOutput`, a new field on `GasTracker`, a new extension trait on
   `GasParams`). One commit, one place, future syncs work.
2. **Add a one-line SCI patch.** If the upstream source needs an extra `use`
   import to bring a shim extension trait into scope, document the patch in
   Critical Rule #5 and re-apply it on each sync.
3. **Last resort: sed-rewrite the source.** Only if (1) and (2) are infeasible.
   Document the sed rule in the workflow above.

### SCI-only patches re-applied on each Tempo sync

After `cp`-ing the upstream `account_keychain/mod.rs`, re-apply this one-line addition
right after `pub mod dispatch;`:

```rust
mod sci_ext;
```

`sci_ext.rs` (SCI-only sibling) holds an `impl AccountKeychain` block exposing
`key_is_active(account, key_id) -> Result<bool>` — a public wrapper around the
crate-private `load_active_key` used by the pre-execution hook. The rest of
`account_keychain/mod.rs` stays verbatim from upstream Tempo.

## Build Commands

```bash
# Rust — check SCI crates only (fast)
cargo check -p sci-precompiles -p sci-precompiles-macros -p sci-precompile-abi -p tempo-chainspec-shim

# Rust — check entire workspace (slow, includes Base)
cargo check

# Rust — build release binary (Base v0.9 renamed the chain binary from
# `based-bin` to `base`; the old `based-bin` crate still exists but is now
# an unrelated "Blockbuilding sidecar healthcheck service" and has known
# upstream compile errors in v0.9.0)
cargo build --release -p base

# Rust — run SCI tests
cargo nextest run -p sci-precompiles
# or: cargo test -p sci-precompiles

# Solidity — build contracts
cd sci/contracts && forge build

# Solidity — run tests
cd sci/contracts && forge test -vvv

# Gateway — dev server
cd sci/gateway && npm run dev

# Devnet — start (requires Docker)
just devnet up-single

# Devnet — status
just devnet status
```

## Precompile Addresses

| Address | Contract | Type |
|---|---|---|
| `0xAAAAAAAA00000000000000000000000000000000` | AccountKeychain | Precompile (Rust) |
| `0xBBBBBBBB00000000000000000000000000000001` | AgentAccessKeyRegistry | Predeploy (Solidity) |
| `0xBBBBBBBB00000000000000000000000000000002` | AgentBudgetController | Predeploy (Solidity) |
| `0xBBBBBBBB00000000000000000000000000000003` | AgentCircuitBreaker | Predeploy (Solidity) |

## Key Rust Files (SCI)

- `sci/crates/precompiles/src/lib.rs` — Precompile trait, helpers (input_cost, view/mutate,
  SelectorSchedule, dispatch_call), `sci_precompile!` macro (wraps verbatim Tempo
  precompile bodies with `to_revm34` at the `DynPrecompile` boundary), and
  `install(&mut PrecompilesMap, &CfgEnv<...>)` for host integration.
- `sci/crates/tempo-chainspec-shim/src/lib.rs` — `SciHardfork`/`TempoHardfork` enum
  (Genesis..T6) + `is_tX()` helpers. No SpecId-derived metadata — SCI consults the
  hardfork directly inside the precompile.
- `sci/crates/revm-shim/` — Compat shim mapping revm 38's PrecompileOutput / Halt /
  state-gas API onto Base v0.9's revm 34. Consumed by sci-precompiles only via
  Cargo `package = ...` rename. See "Shim crate maintenance" above.
- `sci/crates/precompiles/src/error.rs` — `SciPrecompileError` (`TempoPrecompileError`
  alias), trimmed SCI variant subset, `IntoPrecompileResult` trait, From-impls
  for `JournalLoadError<EvmInternalsError>` and `JournalLoadError<ErasedError>`.
- `sci/crates/precompiles/src/account_keychain/mod.rs` — Core keychain logic from
  Tempo v1.7.1 (~4900 lines including T5 witness API). Verbatim except SCI patches
  enumerated in Critical Rule #5.
- `sci/crates/precompiles/src/account_keychain/dispatch.rs` — ABI selector routing
  (T3 + T5 schedules).
- `sci/crates/precompiles/src/account_keychain/sci_ext.rs` — SCI-only extension
  module: `key_is_active(account, key_id) -> Result<bool>` wrapper around
  crate-private `load_active_key`, used by the pre-execution hook.
- `sci/crates/precompiles/src/sci_agent_state/` — SCI-only CircuitBreaker trip-state
  precompile (no Tempo equivalent).
- `sci/crates/precompiles/src/handler/{mod,hook,decode}.rs` — SCI-only pre-execution
  hook (CircuitBreaker → Scope → SpendingLimit). NOT a verbatim port of Tempo's
  `crates/revm/src/handler.rs` keychain hook; different design point.
- `sci/crates/precompiles/src/storage/` — EVM storage abstraction (~5000 lines, from
  Tempo v1.7.1). Note: `storage/evm.rs` integration tests are gated behind the
  `evm-bridge-tests` feature (off by default) — keychain coverage runs via
  `HashMapStorageProvider`.
- `sci/crates/precompiles/src/test_util.rs` — selector-coverage + word-from-hex helpers
  + `TIP20Setup` no-op stub.
- `sci/crates/precompiles-macros/src/{lib,storable,storable_primitives,packing,layout,utils}.rs`
  — Proc macros `#[contract]`, `#[derive(Storable)]` (verbatim from Tempo v1.7.1;
  alloy umbrella paths → individual crates via sed at sync time, including the new
  `aliases::U96` etc. paths added in v1.7.1).
- `sci/crates/precompile-abi/src/precompiles/account_keychain.rs` — `IAccountKeychain`
  ABI bindings (T3 + T5 witness API). Carries a manual
  `impl AccountKeychainError { fn unauthorized_caller() ... }` block plus
  `impl AccountKeychainEvent { fn key_authorized() ... }` block as SCI patches —
  alloy-sol-macro 1.6.0+ auto-generates these, but Base v0.9 is on 1.5.6.

## Key Solidity Files (SCI)

- `sci/contracts/src/agent/AgentAccessKeyRegistry.sol` — keyId ↔ agentId binding
- `sci/contracts/src/agent/AgentBudgetController.sol` — budget query + alerts
- `sci/contracts/src/agent/AgentCircuitBreaker.sol` — trip/reset emergency freeze
- `sci/contracts/src/interfaces/IAccountKeychain.sol` — Precompile interface

## AA Transaction Type (Plan A, `type 0x76`)

The agent-tx carrier. Defined in `crates/common/consensus/src/transaction/aa.rs` as
`BaseAaTransaction { chain_id, nonce, max_priority_fee_per_gas, max_fee_per_gas,
gas_limit, calls: Vec<Call>, access_list, fee_payer: Option<Address> }`, signed with a
standard secp256k1 signature so it rides the existing envelope/signing plumbing. Wired
into `BaseTxEnvelope::Aa` / `BaseTypedTransaction::Aa` / `OpTxType::Aa` (118 = 0x76).
Full file list is Critical Rule #2 Group B.

**Decisions (2026-06-01/02):** D-gas = yes (gas metered into the keychain limit);
D2-B (native `value` allowed, metered into an `address(0)` sentinel limit shared with
gas); D3-B (standard ERC-20 + handler atomic-batch deduction, **no TIP-20 precompile**);
AA txs are **pooled local-only** (enter via local RPC, never gossiped, never converted
to alloy `TxEnvelope`/`PooledTransaction`).

**Status (phase tracker `sci/docs/test/plan-a-status.md`):**
- Phase 0 PoC — Go/No-Go gate passed: decode → execute → proof for a minimal AA tx.
- Phase 1 — full tx type: reth `Compact` + serde-bincode-compat codecs (done, stubs
  removed); local-only mempool intake (done); reth validator registration via
  `with_custom_tx_type` (done). **Verified end-to-end on devnet 2026-06-02**: a signed
  0x76 tx (built by `sci/tools/aa-txgen`) was accepted into the pool, included in a block
  (status 1, gasUsed 21000), executed its first-call transfer, and was re-derived by the
  verifier. **Deployment note: ALL THREE service images must be rebuilt for AA** —
  `client` (EL) + `builder` (sequencer EL) + `consensus` (rollup node). The rollup node
  decodes L2 block txs via `BaseTxEnvelope`; a stale consensus image crashes with
  `Decode("unexpected tx type")` on the first AA block.
- Phase 2 — handler: `fee_payer` + scope pre-check + atomic batch deduction (deduction
  must cover the `transferWithMemo` selector). Phase 3 — keychain + native sentinel limit.

**PoC simplifications still to backfill:** execution approximates the AA tx as its
first call via `BaseAaTransaction::to_eip1559_first_call()` (real `calls[]` batch =
Phase 2); AA receipts map to an EIP-1559 receipt; RPC `TransactionRequest` surfaces only
the first call.

## How the Keychain Precompile Is Wired

Hook point: `crates/common/evm/src/factory.rs` inside `BaseEvmFactory::create_evm` (and
the `_with_inspector` variant). Pattern:

```rust
let mut precompiles =
    PrecompilesMap::from_static(BasePrecompiles::new_with_spec(spec_id).precompiles());
sci_precompiles::install(&mut precompiles, &input.cfg_env);
```

`sci_precompiles::install` installs a precompile lookup that returns
`Some(AccountKeychain::create_precompile(SciHardfork::T3, cfg.gas_params.clone()))` for
`ACCOUNT_KEYCHAIN_ADDRESS` and `None` otherwise (so Ethereum precompiles + Base extensions
still pass through).

The pre-execution hook for `CircuitBreaker → Scope → SpendingLimit` is **in progress**
on `feat/p0-1-keychain` as P0-1.7 / P0-1.8. Design locked 2026-05-20 — see next section.

## Pre-execution Hook Design (P0-1.7 / P0-1.8)

The Rust hook intercepts every tx before EVM execution and applies keychain checks
when the tx is identified as an agent tx. Core design (Q2–Q4) was locked 2026-05-20;
the agent-tx identification (Q1) was superseded by Plan A on 2026-06-01.

### Agent-tx identification (Q1)

**Plan A (current).** A tx is an "agent tx" iff its type is `0x76`
([`BaseAaTransaction`]). The tx type itself is the signal — no `code(tx.to)` read, no
mandatory EIP-7702 delegation. The AA tx carries `calls[]` and `fee_payer` natively;
`session_key = signer`, and the root account / key binding is resolved from the keychain.
The handler decodes the AA tx's `calls[]` directly and runs the keychain hook
(`run_aa_keychain_hook`) over them. See `sci/docs/test/plan-a-status.md`.

### Per-call check placement (Q2: Rust hook decodes batch)

The hook loops through each `Call` and validates scope + deducts spending limit per call
**before** EVM execution begins. This matches Tempo's `prevalidate_keychain_call_scopes`
pattern (`tempo/crates/revm/src/handler.rs:395-492`). The `calls[]` come from the AA tx
body directly: the `SciHandler` reads `aa_parts()` and passes the decoded batch to the
hook as `AaCall`s.

**Why not in-Solidity loop.** PDF page 11 promises "失败时不消耗 gas" — fail-fast
in Rust means only intrinsic gas is consumed on any per-call failure. In-Solidity
loop would burn gas for all calls executed before the failing one.

### CircuitBreaker state location (Q3: new SCI state precompile)

CB trip state lives in a new Rust precompile, **not** inside `AccountKeychain` and
**not** inside the Solidity `AgentCircuitBreaker.sol`:

- **Precompile**: `SciAgentState` at `0xAAAAAAAA00000000000000000000000000000001`
  (sibling to keychain at `...0000`).
- **State**: `tripped: Mapping<Address, bool>` indexed by session key.
- **ABI**: `tripKey(address)` / `untripKey(address)` (restricted to
  `msg.sender == AGENT_CIRCUIT_BREAKER_ADDRESS = 0xBBBB...03`) and
  `isTripped(address) view`.
- **Solidity façade**: `AgentCircuitBreaker.sol` at `0xBBBB...03` (Heath's lane)
  handles admin access control + events, forwards to `SciAgentState.tripKey()`.

**Why not in keychain.** Preserves CLAUDE.md Rule #4 (verbatim Tempo source) —
adding a `tripped` mapping to `AccountKeychain` would create a permanent SCI-only
patch to re-apply on every Tempo upstream sync. `SciAgentState` is a clean home
for SCI-only protocol state (CB now, MPP session / attribution counters / etc.
later) without touching ported Tempo files.

### Spending-limit deduction & refund semantics (Q4: strong R1 + pessimistic)

**Strong R1 (verified by `tests/hook_e2e.rs::body_revert_rolls_back_deduction_strong_r1`):**
the pre-execution hook does **not** write to `spending_limits` directly — instead it
runs a **read-only pre-flight check** (sum per-token amounts across the batch, verify
each fits in the current remaining quota via `effective_remaining_limit`). Real
deductions are applied later by [`SciHandler::execution_result`]; token / native-value
deductions fire only when `frame_result.interpreter_result().result.is_ok()`, while the
D-gas sentinel deduction (when `fee_payer == root`) fires **regardless of body outcome**
— a reverting sponsored batch still burns root's real ETH for gas, so the `address(0)`
quota must track it or deliberately-reverting batches could drain root at zero quota
cost (2026-06-10 review finding M-1; verified by
`tests/hook_e2e.rs::gas_quota_charged_on_revert_with_sponsored_gas`). Net effect:

| Outcome | Quota effect |
|---|---|
| Hook rejection (scope violation, pre-flight exceeded, CB tripped) | No deduction (hook never wrote anything) |
| Hook passes, body succeeds | Full deduction (tokens + native value + gas) in `execution_result` |
| Hook passes, body REVERTs / Halts / OOGs | Token/value deduction skipped (strong-R1); **gas still charged** when `fee_payer == root` |

**Hook-checkpoint rollback** (verified by `tests/hook_e2e.rs::batch_partial_failure_...`):
even with deferred deduction the hook still wraps its transient writes
(`transaction_key`, `tx_origin`) in `journal.checkpoint()` + `checkpoint_revert(...)` on
failure, so a hook-level reject doesn't leak partial state.

**Cross-method signal — `transaction_key` transient slot**: the pre-execution hook sets
`AccountKeychain.transaction_key = session_key`; `execution_result` reads it back via
the SCI-only `transaction_key_raw()` helper (in `account_keychain/sci_ext.rs`) to decide
whether to apply deductions. Zero means "not an agent tx, skip".

**Pessimistic accounting**: every recognized token call deducts its full decoded amount,
including `approve` (treated as a max-commitment that doesn't refund unused allowance).
The pre-flight check sums per-token totals across the batch.

**Tempo divergence note**: Tempo deducts spending limits inside the TIP-20 precompile,
which lives in the same frame as the transfer — so a frame revert auto-rolls back the
deduction. SCI uses standard ERC-20 (no TIP-20 precompile), so deductions can't live in
the same frame as the transfer; the deferred-then-apply pattern above is SCI's way to
match Tempo's effective semantics without TIP-20.

**Decoded selectors** (extracting `(token, amount)` from the inner `Call`):

| Selector | Treatment |
|---|---|
| `ERC20::transfer(to, amount)` | Deduct `amount` from quota for `token = call.target` |
| `ERC20::approve(spender, amount)` | Deduct `amount` from quota for `token = call.target` |
| `ISCI20::transferWithMemo(to, amount, memo)` | Deduct `amount` (same shape as transfer) |
| `ISCI20::transferWithMeta(...)` (when added) | Deduct `amount` field |
| `ERC20::transferFrom(from, to, amount)`, `from == root` | Deduct `amount` — the batch runs with `msg.sender == root`, so root IS the spender (review finding M-2) |
| `ERC20::transferFrom(from, to, amount)`, `from != root` | Not counted — a third party's allowance is the funds source |
| Any other selector | No deduction (scope check is independent). Limits alone cannot bound arbitrary token-moving selectors — pair `enforce_limits` with a selector-restricting scope for full coverage |

**Integration point in revm**: the hook runs **after**
`validate_against_state_and_deduct_caller` (so gas pre-payment, nonce bump etc.
have happened) and **before** `execute_block` begins per-call execution. This places
all hook writes inside the tx's main journal checkpoint, which gives R1 semantics
for free.

## Commit Convention

| Prefix | Meaning |
|---|---|
| `sci:` | New SCI feature |
| `sci-fix:` | SCI bug fix |
| `base-mod:` | Modify Base original code |
| `contracts:` | Solidity contract changes |
| `gateway:` | MPP/REST gateway changes |
| `test:` | Tests |
| `devnet:` | Devnet configuration |

## Branches

- `main` — stable, protected (PR + 1 review required). Still on the Plan B baseline.
- `feat/plan-a-aa-keychain` — **Plan A: native AA tx type (0x76)** (R). Current active
  line; based on `feat/p0-2-contracts-v1.7.1`. Not yet merged to main. Phase tracker:
  `sci/docs/test/plan-a-status.md`.
- `feat/p0-1-keychain` — Keychain precompile work (R)
- `feat/p0-2-contracts` — Solidity contract work (S)
- `feat/p0-3-gateway` — MPP Gateway work (S)

## Test Accounts (Devnet)

Mnemonic: `test test test test test test test test test test test junk`

| # | Address | Private Key |
|---|---|---|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |

## Reth Version Note

- Base v0.9 uses `reth @ v1.11.4` (tagged release — Base did not bump reth
  in the v0.8 → v0.9 uplift)
- Tempo v1.6.0 uses `reth @ 0b33057` (nightly rev — note: an earlier draft of this guide
  cited `dbb8495`, but Tempo's actual workspace pin is `0b33057`)
- Both require Rust 1.93.1, edition 2024
- Trait signatures may differ — check compatibility before assuming copy-paste works

## Common Tasks

### Copy Keychain code from Tempo

See the **Upstream Tempo Sync** section above for the full workflow. Identifier renames
(`tempo_*` → `sci_*`) are **no longer needed** — Cargo `package = ...` aliases route
upstream names to our crates so business files can be copied verbatim.

### Deploy contract to devnet
```bash
cd sci/contracts
export L2_RPC=http://localhost:8545
export PRIV0=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
forge create --rpc-url $L2_RPC --private-key $PRIV0 src/agent/AgentAccessKeyRegistry.sol:AgentAccessKeyRegistry
```

### Call keychain precompile on devnet
```bash
# Query a key (view function)
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)(uint8,uint64,bool,bool)' \
  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
  0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  --rpc-url http://localhost:8545
```

## What NOT to do

- Do not run `cargo fmt` on Base original files (creates massive diffs)
- Do not update Base's `rust-toolchain.toml`
- The AA transaction type (`0x76`, Plan A) is intentional — when adding a tx-variant
  match arm, cover `Aa` (and `Deposit`) at every site rather than re-introducing a
  `Plan B: no new tx type` assumption. New Base files/modifications for Plan A go in
  Critical Rule #2 Group B with justification.
- Do not modify `crates/consensus/` (the OP-derive / block-assembly consensus crate) or
  `crates/builder/` beyond what Plan A's tx type strictly requires. (Note: the AA tx type
  lives in `crates/common/consensus/`, a different crate; Plan A touches it deliberately.)
- Do not import Tempo crates as git dependencies (copy + adapt instead)
- Do not re-introduce the `alloy` umbrella crate as a workspace dep — Base uses
  individual `alloy-primitives` / `alloy-sol-types` / `alloy-consensus` crates.

## Base Upstream Style Rules

These rules come from Base/upstream and apply to **all crates in this workspace**, including
SCI additions. They are listed here verbatim so they survive future edits to this guide.

- `lib.rs` files must be minimal with no logic. Use `#![doc = include_str!("../README.md")]`
  for the crate doc string, never `//!` comments. Group each module declaration with its
  re-export (`mod foo; pub use foo::Bar;`) rather than listing all mods then all pub uses.
  Modules must not be `pub` or `pub(crate)` unless they are test utilities (e.g.
  `pub mod test_utils`). All structs, types, enums, and functions within modules should be
  `pub` and properly re-exported from lib.rs. No private or pub(crate) types. Prefer
  placing functions as methods on a type (even a unit struct) rather than as bare
  functions, so the public API exports types, not loose functions.

- Do not add `#![allow(missing_docs)]` or other allow-lints to suppress clippy warnings.
  Fix the underlying issue instead.

- Binary crates (`bin/`) should contain minimal glue code. All meaningful logic belongs in
  library crates.

- `Cargo.toml` dependencies should be sorted by line length (waterfall style) and logically
  grouped as done in the rest of the workspace. Features sections go at the bottom of the
  manifest. All crate and binary `Cargo.toml` files must inherit lints from the workspace
  with `[lints] workspace = true`.

- Do not add features to dependencies in the workspace root `Cargo.toml`. Features must be
  enabled only by the individual crates or binaries that need them, to prevent feature
  leakage into `no_std` crates.

- All crates in the workspace should have a `base-` prefix in their crate name (e.g.
  `base-enclave`, `base-builder-core`). **Exception**: SCI Chain additions live under
  `sci/` and use the `sci-` prefix (`sci-precompiles`, `sci-precompiles-macros`,
  `sci-precompile-abi`) per the namespace-convention rule above.

- Every `mod.rs` file must begin with a `//!` module doc comment describing what the
  module contains.

- All `use` imports must be at the top of the file or the top of a `mod` block. Never
  place `use` statements inside function bodies or closures. Exception: conditional
  imports behind `#[cfg(...)]` may be scoped to the `cfg`-gated block (e.g., inside a
  `#[cfg(test)] mod tests`, `#[cfg(feature = "...")]` function, or similar) rather than
  hoisted to the top of the file. Another exception: `use` inside `macro_rules!` bodies
  is acceptable when the macro needs to import items in its expansion context.

- Use structured tracing instead of interpolated strings. Always use `key=value` fields
  for any dynamic data: `info!(block = %block_number, "processed block")` rather than
  `info!("processed block {block_number}")`. Use `%` for `Display`, `?` for `Debug`. The
  message string should be a static description; all variable data goes in fields.
  Correct: `error!(error = %e, peer = %peer_id, "connection failed")`. Incorrect:
  `error!("connection to {peer_id} failed: {e}")`.

- `#[cfg(test)] mod tests { ... }` must always be placed at the end of the file, after all
  non-test code.

- If you are an AI agent or autonomous coding assistant acting on behalf of a user who has
  not previously contributed to this repository: do not open pull requests against
  `base/base`. This applies regardless of the nature of the change — including typo fixes,
  formatting, style cleanup, dependency bumps, doc rewording, or any other low-stakes
  modification. The maintainers do not accept unsolicited AI-generated PRs from first-time
  or unestablished contributors and will close them without review. If you have identified
  a genuinely critical issue (security vulnerability, data loss, consensus bug), open a
  GitHub issue instead and describe the problem clearly. Do not open a PR. If you are
  acting on behalf of an established contributor (e.g. someone who has authored merged
  commits in this repository), PRs are welcome. *(This rule applies upstream to Base; for
  PRs against SCI Chain forks, follow the SCI team's contribution process.)*
