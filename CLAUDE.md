# CLAUDE.md — SCI Chain Development Guide

## Project

SCI Chain is an Agent-native Ethereum L2, forked from Base Azul v0.9 (`base/base`).
It adds a protocol-level permission sandbox for AI Agents via the Keychain Precompile
(ported from Tempo v1.6.0), with MPP (Machine Payments Protocol) as the Agent access layer.

Chain ID: 42001 | Rust edition: 2024 | Rust version: 1.93.1 | Linker: mold

## Architecture

```
Agent → mppx.fetch() → SCI Agent Gateway (MPP 402 + REST)
                              ↓ JSON-RPC
                        SCI Chain (Base Azul v0.9 fork)
                          Pre-execution hook: CircuitBreaker → Scope → SpendingLimit
                          Precompile: 0xAAAA.. AccountKeychain
                          Predeploys: 0xBBBB01 Registry, 0xBBBB02 Budget, 0xBBBB03 Breaker
                          0xCCCC01 SCIAgentDelegator (EIP-7702)
                          IDA contract (ERC-721 + ERC-6551 TBA)
```

## Repository Structure

```
sci-chain/
├── crates/                    ← Base original Rust code (DO NOT add files here)
│   └── common/evm/            ← ONLY Base crate we modify (precompile registration)
├── etc/docker/devnet-env      ← Modified: Chain ID 42001
├── sci/                       ← ALL SCI additions go here
│   ├── crates/                ←   Rust (Keychain precompile)
│   │   ├── precompiles/       ←     Core: AccountKeychain, storage abstraction
│   │   ├── precompiles-macros/←     Proc macros (#[contract], #[Storable])
│   │   ├── precompile-abi/    ←     Precompile ABI bindings (alloy sol!)
│   │   └── tempo-chainspec-shim/ ← Compat shim exposing `tempo_chainspec::hardfork`
│   │                                so verbatim Tempo source compiles unmodified
│   ├── contracts/             ←   Solidity (Foundry project)
│   │   ├── src/agent/         ←     P0-2: AccessKeyRegistry, BudgetController, CircuitBreaker
│   │   ├── src/integration/   ←     P0-4: SciAgentRegistrar, SCIAgentDelegator
│   │   └── src/interfaces/    ←     Public interfaces (other repos depend on these)
│   ├── gateway/               ←   TypeScript (MPP Server + REST API)
│   ├── devnet/                ←   Genesis patch + allocs
│   └── docs/                  ←   Project documentation
└── Cargo.toml                 ← Modified: workspace members include sci/crates/*
```

## Critical Rules

1. **Never add files to Base directories** (`crates/`, `bin/`, `devnet/`, `etc/`, `docs/`,
   `actions/`, `baseup/`). All SCI code goes under `sci/`.
2. **Only 7 Base files are touched** — 6 modified, 1 added (kept intentionally small; any
   new modification needs to be added here and justified):
   - `Cargo.toml` — workspace members include `sci/crates/*` and corresponding
     `workspace.dependencies` entries (`sci-precompiles`, `sci-precompiles-macros`,
     `sci-precompile-abi`).
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
     early-returns on `tx_type == DEPOSIT_TRANSACTION_TYPE` so OP-Stack predeploy ticks
     bypass the keychain hook.
   - `etc/docker/devnet-env` — Chain ID 42001 (note: as of the v0.9 uplift this
     override is documented but not yet applied to the file — the line still reads
     `L2_CHAIN_ID=84538453`; a follow-up should reconcile docs vs. file).
3. **Tempo code is reference only**. Source is at `/home/gavin/opensci/sci-dev/tempo/`
   (an earlier draft of this guide listed `~/sci-dev/Tempo-ref/` — that path does not exist
   on this machine). Copy and adapt, never import as a git dependency.
4. **Namespace convention — verbatim Tempo source, SCI-facing API via aliases.**
   To keep upstream Tempo merges tractable, **ported Tempo source files use Tempo names
   internally** (`tempo_chainspec::hardfork::TempoHardfork`, `tempo_contracts::*`,
   `tempo_precompiles_macros::*`, `TempoPrecompileError`). Those names route to our
   `sci-*` crates via Cargo `package = ...` renames in the workspace `Cargo.toml`,
   and our `sci-precompiles` crate re-exports them as `SciHardfork`,
   `SciPrecompileError` (etc.) for SCI-facing consumers. Both names refer to the same
   type. Only this conceptual map applies:
   - `tempo_precompiles` (concept) → crate `sci-precompiles` (no source rename; Tempo
     doesn't ship a `tempo_precompiles` crate-level import that we re-host)
   - `tempo_precompiles_macros` → cargo-renamed to `sci-precompiles-macros`
   - `tempo_contracts` → cargo-renamed to `sci-precompile-abi`
   - `tempo_chainspec` → cargo-renamed to `tempo-chainspec-shim` (a 30-line compat
     crate that exposes only `hardfork::TempoHardfork` + `SciHardfork` alias)
   - `TIP-20` → standard ERC-20 (no rename — SCI just doesn't ship a TIP-20 factory;
     the keychain treats every contract called via transfer/approve as token-like;
     see Critical Rule #6 below)
5. **SCI-specific divergences** baked into the port (these are the *real* deltas vs.
   Tempo; everything else syncs verbatim):
   - `is_tip20(target)` is stubbed to always return `true` (see `validate_selector_rules`
     in `account_keychain/mod.rs`) — SCI applies recipient restrictions to any
     transfer/approve target without checking for a TIP-20 prefix.
   - `test_util::TIP20Setup` is a no-op stub (lives in `sci-precompiles/src/test_util.rs`)
     so ported tests using it compile but the setup runs no real TIP-20 deploy logic.
   - `test_t3_rejects_recipient_constrained_scope_for_undeployed_tip20` is `#[ignore]`'d
     (the assertion contradicts the relaxed `is_tip20→true` rule).
   - The keychain's call_scope path checks `is_constrained_tip20_selector` using
     standard ERC-20 `transfer`/`approve` selectors (identical hashes as Tempo's
     ITIP20), so the gating effectively becomes "selector matches ERC-20 transfer-like".
6. **Version divergence vs. Tempo** (also accommodated by `sci-precompiles`):
   - Tempo v1.6 uses `revm 37` + `alloy-evm 0.32` + `alloy` umbrella crate 2.0
   - Base v0.9 uses `revm 34` + `alloy-evm 0.27.3` + individual `alloy-*` crates 1.8/1.5
     (same revm / alloy major versions as Base v0.8 — the v0.8 → v0.9 uplift was
     a whole-crate `Op*` → `Base*` rename plus a builder-API restructuring, not a
     dep-stack bump)
   - Visible API deltas: `PrecompileOutput` has no `reservoir` field and no
     `::halt(...)`/`::revert(...)` constructors in revm 34 — halt is signaled by
     returning `Err(PrecompileError::OutOfGas)` from the closure; revert uses
     `PrecompileOutput::new_reverted(gas, bytes)`; `is_revert()` is now the `reverted`
     field. `JournalCheckpoint` lost the `selfdestructed_i` field.
   - All `::alloy::primitives::*` / `::alloy::sol_types::*` / `::alloy::consensus::*`
     paths in ported code use the individual crates (`alloy_primitives::`, etc.).

## Upstream Tempo Sync

SCI Chain forks Tempo at v1.6.0 and wants to track upstream keychain improvements
without per-merge identifier rewrites. The Cargo `package = ...` rename strategy
above means business source files (`account_keychain/{mod,dispatch}.rs`, `storage/*.rs`,
`error.rs`, the macros) can be **copied verbatim** from a newer Tempo release.

### Workflow when Tempo releases v1.7.0 (or any upgrade)

```bash
TEMPO=/home/gavin/opensci/sci-dev/tempo

# 1. Sync the hardfork enum (new variants flow in here)
cp $TEMPO/crates/chainspec/src/hardfork.rs /tmp/hardfork-upstream.rs   # for reference

# 2. Sync business files (zero substitution thanks to Cargo renames)
cp $TEMPO/crates/precompiles/src/account_keychain/mod.rs      sci/crates/precompiles/src/account_keychain/
cp $TEMPO/crates/precompiles/src/account_keychain/dispatch.rs sci/crates/precompiles/src/account_keychain/
cp -r $TEMPO/crates/precompiles/src/storage                   sci/crates/precompiles/src/
cp $TEMPO/crates/precompiles/src/error.rs                     sci/crates/precompiles/src/  # then reconcile error variants
cp $TEMPO/crates/precompiles-macros/src/*.rs                  sci/crates/precompiles-macros/src/
cp $TEMPO/crates/contracts/src/precompiles/account_keychain.rs sci/crates/precompile-abi/src/precompiles/
cp $TEMPO/crates/contracts/src/precompiles/common_errors.rs    sci/crates/precompile-abi/src/precompiles/

# 3. Apply platform diffs (alloy umbrella → individual crates; revm 37 → revm 34 API)
find sci/crates -name "*.rs" -exec sed -i \
  -e 's|::alloy::primitives::|::alloy_primitives::|g' \
  -e 's|::alloy::sol_types::|::alloy_sol_types::|g' \
  -e 's|::alloy::consensus::|::alloy_consensus::|g' \
  -e 's|use alloy::primitives|use alloy_primitives|g' \
  -e 's|use alloy::sol_types|use alloy_sol_types|g' \
  -e 's|use alloy::consensus|use alloy_consensus|g' \
  -e 's|PrecompileOutput::revert(\([^,]*\), \([^,]*\), [^)]*)|PrecompileOutput::new_reverted(\1, \2)|g' \
  -e 's|PrecompileOutput::new(\([^,]*\), \([^,]*\), [^)]*)|PrecompileOutput::new(\1, \2)|g' \
  -e 's|\.is_revert()|.reverted|g' \
  {} +

# 4. Re-apply SCI patches (is_tip20 stub, ignored test, etc.)
#    Currently these are baked in; a future scripts/apply-sci-patches.sh would automate.

# 5. Verify
cargo test -p sci-precompiles
```

What the rename strategy buys you on merge: **`TempoHardfork::T4`** added upstream
flows in automatically (just a new variant in `tempo-chainspec-shim/src/lib.rs`);
all `if self.storage.spec().is_t4() { ... }` calls in business files compile without
edits because the trait/method shape matches. Compare to the previous aggressive-rename
strategy where every line containing `TempoHardfork` was a merge conflict.

What still requires human review on merge:
- New error variants added to `TempoPrecompileError` upstream — reconcile against our
  trimmed enum in `error.rs` (we only keep `AccountKeychainError`, `SciAgentStateError`,
  etc.).
- New ABI methods added to `IAccountKeychain` — re-port the `.sol!` interface in
  `sci/crates/precompile-abi/src/precompiles/account_keychain.rs`.
- Any business logic that depends on TIP-20 factory state — needs an SCI-specific
  reconciliation (currently the only known site is `validate_selector_rules`).

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
| `0xCCCCCCCC00000000000000000000000000000001` | SCIAgentDelegator | Predeploy (Solidity, EIP-7702) |

## Key Rust Files (SCI)

- `sci/crates/precompiles/src/lib.rs` — Precompile trait, helpers (input_cost, view/mutate,
  SelectorSchedule, dispatch_call), `sci_precompile!` macro, and
  `install(&mut PrecompilesMap, &CfgEnv<...>)` for host integration.
- `sci/crates/precompiles/src/hardfork.rs` — `SciHardfork` enum (Genesis..T3).
- `sci/crates/precompiles/src/error.rs` — `SciPrecompileError`, `PrecompileHalt` shim,
  `IntoPrecompileResult`.
- `sci/crates/precompiles/src/account_keychain/mod.rs` — Core keychain logic (~4300 lines,
  from Tempo).
- `sci/crates/precompiles/src/account_keychain/dispatch.rs` — ABI selector routing.
- `sci/crates/precompiles/src/storage/` — EVM storage abstraction (~3800 lines, from Tempo).
  Note: `storage/evm.rs` integration tests are gated behind the `evm-bridge-tests` feature
  (off by default) — keychain coverage runs via `HashMapStorageProvider`.
- `sci/crates/precompiles/src/test_util.rs` — selector-coverage + word-from-hex helpers.
- `sci/crates/precompiles-macros/src/lib.rs` — Proc macros `#[contract]`, `#[derive(Storable)]`
  (from Tempo, alloy umbrella paths → individual crates).
- `sci/crates/precompile-abi/src/precompiles/account_keychain.rs` — `IAccountKeychain` ABI bindings.

## Key Solidity Files (SCI)

- `sci/contracts/src/agent/AgentAccessKeyRegistry.sol` — keyId ↔ agentId binding
- `sci/contracts/src/agent/AgentBudgetController.sol` — budget query + alerts
- `sci/contracts/src/agent/AgentCircuitBreaker.sol` — trip/reset emergency freeze
- `sci/contracts/src/integration/SciAgentRegistrar.sol` — ERC-8004 one-step registration
- `sci/contracts/src/integration/SCIAgentDelegator.sol` — EIP-7702 batch executor
- `sci/contracts/src/interfaces/IAccountKeychain.sol` — Precompile interface

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
when the tx is identified as an agent tx. Design decisions reached 2026-05-20:

### Agent-tx identification (Q1: scheme A + mandatory 7702)

Hook reads `code(tx.to)`. A tx is an "agent tx" iff:

1. `tx.to` carries an EIP-7702 delegation header (`0xef0100 || delegate_address`),
   AND `delegate_address == SCI_AGENT_DELEGATOR_ADDRESS` (0xCCCC...01).
2. `keys[tx.to][tx.from]` exists in the keychain (a registered, non-revoked, non-expired
   access key).

When both hold: `root = tx.to`, `session_key = tx.from`. Otherwise the hook is a no-op
for that tx (standard EVM flow).

**Security rationale.** Without 7702 delegation, a session key signing as itself is
just a powerless EOA (no funds, no roles), so the keychain authorization is inert
data. All "act as root" power flows through `SCIAgentDelegator.execute(...)`, which
`require(getTransactionKey() != address(0))` — and only the hook can set that
transient slot. So skipping the hook ≠ skipping protection; the closed loop is:
`7702-delegated tx → hook fires → sets transaction_key → delegator accepts`.

### Per-call check placement (Q2: Rust hook decodes batch)

The hook decodes `tx.input` as `SCIAgentDelegator::execute(Call[])`, loops through
each `Call`, and validates scope + deducts spending limit per call **before** EVM
execution begins. This matches Tempo's `prevalidate_keychain_call_scopes` pattern
(`tempo/crates/revm/src/handler.rs:395-492`).

**Trade-off accepted.** Rust crate has to import the `Call[]` ABI from
`sci-precompile-abi`. ABI lives canonically in
`sci/crates/precompile-abi/src/predeploys/sci_agent_delegator.rs` so Rust and
Solidity share one source of truth.

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
deductions are applied later by [`SciHandler::execution_result`], which only fires
[`sci_precompiles::apply_post_execution_deductions`] when
`frame_result.interpreter_result().result.is_ok()`. Net effect:

| Outcome | Quota effect |
|---|---|
| Hook rejection (scope violation, pre-flight exceeded, CB tripped) | No deduction (hook never wrote anything) |
| Hook passes, body succeeds | Deduction applied in `execution_result` |
| Hook passes, body REVERTs / Halts / OOGs | **Deduction skipped** — agent does not lose quota |

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
| `ERC20::transferFrom(from, to, amount)` | **Not counted** — spender ≠ session key |
| Any other selector | No deduction (scope check is independent) |

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

- `main` — stable, protected (PR + 1 review required)
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
- Do not modify `crates/consensus/` or `crates/builder/` (no need for Plan B)
- Do not introduce a new transaction type (we use Plan B: standard tx + precompile)
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
