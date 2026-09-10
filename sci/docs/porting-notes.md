# Porting Tempo's Keychain to SCI Chain

This document is a deep technical writeup of how SCI Chain ported Tempo's keychain
execution model into a Base L2, and why the integration is shaped the way it is. It
covers the three load-bearing pieces — the `AccountKeychain` precompile, the
`revm-shim` compatibility layer, and the pre-execution hook — plus the porting
workflow and the known divergences and pitfalls.

Target audience: anyone who needs to understand or modify the SCI execution layer.

---

## 1. Background

SCI Chain is an agent-native Ethereum L2 forked from Base (`base/base`, Azul v0.9).
The product requirement is a **protocol-level permission sandbox for AI agents**: an
agent holds a session key that can spend a bounded amount per token, within a bounded
call scope, and can be frozen instantly.

The right primitive for this already existed in Tempo (`tempoxyz/tempo`) as the
`AccountKeychain` precompile and the native account-abstraction (AA) transaction type.
At the time (mid-2026), Base had not shipped its own native account abstraction
(EIP-8130) or native token standard (B20), so SCI borrowed Tempo's design rather than
re-implementing it.

The port is not a fork of Tempo — it is a **near-verbatim adaptation of Tempo's business
source, ported to Base's older revm version through a compatibility shim** (plus a
self-written pre-execution hook that Tempo does not have). See
`THIRD_PARTY_NOTICES.md` at the repository root for attribution and the derived-file
list.

### Version map

| Component | SCI Chain | Tempo (port source v1.7.1) |
|---|---|---|
| revm | 34.0.0 | 38.0.0 |
| reth | v1.11.4 (tagged) | nightly rev |
| host chain | Base Azul v0.9 | Tempo L1 |

The 34 → 38 gap is the EIP-8037 / TIP-1016 "state gas + reservoir" release. It changed
the precompile return type, the halt semantics, the gas-accounting interface, and the
journal structure — which is exactly what `revm-shim` absorbs (§3).

---

## 2. Architecture

```
Agent (session key, keychain signature)
   └─ MPP Gateway (JSON-RPC) ──► AA transaction type 0x76
        fields: chain_id, nonce, 1559 fees, calls[], access_list, fee_payer
   │
   ▼ SCI Chain (Base Azul v0.9 fork, chainID 42001)
   ├─ txpool: local-only for AA; keychain-authorized admission for sponsored txs
   ├─ SciHandler (wraps BaseHandler)
   │    1. recover session key from the AA signature
   │    2. fee_payer sponsors gas (pre-fund + reconcile, signer nets zero)
   │    3. run_aa_keychain_hook: Authorization → CircuitBreaker → Scope → SpendingLimit
   │    4. execute_aa_batch: atomic calls[] batch (one journal checkpoint)
   │    5. execution_result: deferred spending-limit deduction
   ├─ Precompile 0xAAAA…00: AccountKeychain (ported from Tempo v1.7.1)
   ├─ Precompile 0xAAAA…01: SciAgentState (SCI-only circuit-breaker state)
   └─ Predeploys 0xBBBB…01/02/03: Registry / Budget / Breaker (Solidity façades)
   ▼
   Multi-proof: Kona fault-proof client / TEE (Nitro) / ZK (Succinct) — three-way consistent
```

An agent transaction is a **native AA transaction (type `0x76`)**: it carries a batch
of `calls[]` that execute as the `root` account, plus an optional `fee_payer` for
sponsored gas. The session key is the transaction signer; the keychain authorizes the
`(root, session_key)` binding and gates the batch.

---

## 3. The `AccountKeychain` precompile

### What it stores

```
keys[root][session_key] → AuthorizedKey {
    expiry,          // session-key expiry
    per-token limit, // spending limit per ERC-20 token
    call scope,      // target → selector → recipient constraints
    enforce_limits,  // whether limits are enforced for this key
}
```

Key types support secp256k1 / P256 / WebAuthn (Passkey). The `AuthorizedKey` struct is
packed into a single storage slot to save SLOAD/SSTORE.

### Why a native precompile instead of a contract

1. **Keys must not live in a contract.** A contract cannot keep a key secret (any
   account can read storage) and cannot make revocation unbypassable. A precompile runs
   in the execution layer and is outside the reach of any contract.
2. **Signature verification must be native.** P256 / WebAuthn verification is
   expensive and hard to meter in EVM bytecode; a precompile does it in native code.
3. **It is metered and provable.** Precompile state is part of the state root, so
   "what the agent spent" is covered by Base's multi-proof (Kona / TEE / ZK) like any
   other state.

### Porting strategy: near-verbatim port + pinned patches

The keychain business source (`account_keychain/{mod,dispatch}.rs`, `storage/*.rs`,
the proc macros, and the ABI bindings) is **adapted** from Tempo v1.7.1 — near-verbatim
for the business logic, with import reordering, revm 34 compatibility work (heaviest in
`storage/evm.rs`), and test-harness adjustments accounting for the remaining diff
(~15% of lines). Two mechanisms keep that diff minimal:

- **Cargo `package =` renames** route upstream identifiers to SCI crates without
  source edits: `tempo_precompiles_macros` → `sci-precompiles-macros`,
  `tempo_contracts` → `sci-precompile-abi`, `tempo_chainspec` → `tempo-chainspec-shim`,
  and `revm` → `sci-revm-shim`.
- **`revm-shim`** absorbs the revm 34 ↔ 38 API gap (§4).

A small, stable list of SCI divergences is re-applied on every sync (see
`CLAUDE.md` Critical Rule #5):

| Divergence | Reason |
|---|---|
| `is_tip20(target)` always returns `true` | SCI ships no TIP-20 factory; any ERC-20 transfer/approve target is treated as token-like |
| `enable_amsterdam_eip8037` hardcoded `false` | SCI's revm 34 `CfgEnv` has no such field; SCI does not adopt EIP-8037 state gas |
| `JournalCheckpoint` drops the `selfdestructed_i` field | revm 34 has no such field |
| Manual `AccountKeychainError`/`Event` constructors | alloy-sol-macro 1.5.6 (Base) does not auto-generate them; 1.6.0+ does |

---

## 4. The `revm-shim` compatibility layer

### The problem

The ported Tempo source is written against revm 38's API. Base Azul v0.9 uses revm 34.
Between them, revm introduced EIP-8037 / TIP-1016 "state gas + reservoir" accounting,
which broke, at the source level:

| # | Breaking change | revm 34 | revm 38 | Shim handling |
|---|---|---|---|---|
| 1 | `PrecompileOutput` fields | `{ gas_used, gas_refunded, bytes, reverted: bool }` | adds `state_gas_used`, `reservoir`; `reverted` → `status: ExecutionStatus` | newtype carries v38 fields; state-gas fields stay `0` |
| 2 | Halt semantics | OOG via `Err(PrecompileError::OutOfGas)` | new `PrecompileHalt` enum + `::halt(reason, reservoir)` | new enum + constructor; folded back to `Err` at the boundary |
| 3 | Constructor signatures | `new(gas, bytes)` | `new`/`revert`/`halt` take a trailing `reservoir` | trailing arg accepted and ignored |
| 4 | `.is_revert()` accessor | field `.reverted` | method `.is_revert()` | newtype provides `.is_revert()` |
| 5 | `interpreter::gas::GasTracker` | absent | added (state-gas ledger) | no-op stub, counters stay `0` |
| 6 | `GasParams` state-gas methods | absent | `code_deposit_state_gas` / `create_state_gas` / `sstore_state_gas` | `GasParamsExt` trait, returns `0` |
| 7 | `JournalCheckpoint` field | no `selfdestructed_i` | adds it | dropped (storage-layer patch) |
| 8 | `CfgEnv.enable_amsterdam_eip8037` | absent | added | hardcoded `false` (storage-layer patch) |
| 9 | alloy packaging | individual `alloy-*` crates | `alloy` umbrella | `sed` sweep at sync time |

The essential nature of this release is that it split "state growth cost" out of
ordinary gas into a separately-metered `state_gas` with a `reservoir`. SCI is an L2
whose state-cost model follows Base, so it does not adopt EIP-8037 — every state-gas
field in the shim is deliberately zeroed, not merely stubbed.

### The mechanism

`sci/crates/revm-shim` is 4 files, ~484 lines:

1. **Cargo `package =` rename (the single wiring point).** Only
   `sci/crates/precompiles/Cargo.toml` declares `revm = { package = "sci-revm-shim" }`,
   so every `use revm::…` *inside `sci-precompiles`* resolves through the shim. Every
   other workspace crate — including `base-common-evm` and the `SciHandler` host —
   keeps depending on real revm 34.

2. **Additive shadowing.** `lib.rs` re-exports all revm 34 submodules verbatim
   (`context`, `handler`, `primitives`, `state`, …) and shadows only the two modules
   revm 38 changed: `precompile` and `interpreter::gas`.

3. **Boundary function `to_revm34()`.** Called inside `sci_precompiles::install` at
   the `DynPrecompile` boundary, it folds the shim's `PrecompileResult` back into
   revm 34's native type:
   - `Halt(OutOfGas)` → `Err(PrecompileError::OutOfGas)`
   - `Halt(Other(msg))` → `Err(PrecompileError::Other(msg))`
   - `Revert` → `Ok(PrecompileOutput { reverted: true, … })`
   - `Success` → `Ok(PrecompileOutput { reverted: false, … })`

### Invariants

- **Additive.** The shim never removes or shadows any revm 34 item except the two
  listed modules. Adding v38 surface here does not perturb any crate that still binds
  to real revm 34.
- **`reservoir = 0`, `amsterdam_eip8037_enabled = false` always.** SCI does not adopt
  EIP-8037 / TIP-1016. If it ever does, the no-op stubs are the place to add real
  semantics.

### What the shim deliberately does *not* do

The shim does **not** touch the opcode interpreter loop — revm 34 and 38 do not differ
at the EVM instruction-set level. The differences are at three API boundaries
(precompile output, state-gas accounting, journal shape). Keeping the shim off the
interpreter is what lets verbatim Tempo source compile unmodified and lets every other
crate use real revm 34 simultaneously.

The work that *is* below the opcode loop — building depth-0 frames for the AA batch,
batch atomicity via journal checkpoints, and fee_payer gas reconciliation — lives in
`SciHandler` (§5), not in the shim.

---

## 5. The pre-execution hook

### Placement

The hook logic lives in `sci/crates/precompiles/src/handler/hook.rs` and is driven by
`crates/common/evm/src/sci_handler.rs`. `SciHandler` wraps `BaseHandler` and overrides
only the two `Handler` trait methods it needs to, delegating everything else verbatim:

- `validate_against_state_and_deduct_caller` — fee_payer sponsorship + keychain gate
- `execution` / `execution_result` / `reimburse_caller` — batch execution, deferred
  deduction, gas-refund routing

Overriding as few methods as possible keeps the upstream-Base sync surface minimal.

### The gate (for AA transactions with `root` set)

```
1. Authorization   keys[root][session_key] must be an active access key
2. CircuitBreaker  the session key must not be tripped (SciAgentState.isTripped)
3. Call scope      each call must satisfy the key's target/selector/recipient rules
4. SpendingLimit   read-only pre-flight: each token's batch total must fit the remaining quota
```

Step 1 is what makes sponsored gas safe: an arbitrary signer cannot act as — or spend
the gas of — an unconsenting `root`.

### Two important design points

**Strong-R1: pre-flight + deferred deduction.** The hook performs only a *read-only*
pre-flight check; the real deduction runs in `execution_result` and only when the EVM
body succeeded. Net effect:

| Outcome | Quota effect |
|---|---|
| Hook rejection (scope / limit / breaker) | no deduction |
| Hook passes, body succeeds | full deduction (tokens + native value + gas) |
| Hook passes, body reverts / halts / OOGs | token/value skipped; **gas still charged when `fee_payer == root`** |

Tempo achieves the same "limits roll back with a reverting transfer" semantics natively
because its deduction happens inside the TIP-20 precompile, in the same frame as the
transfer. SCI uses standard ERC-20 (no TIP-20 precompile), so the deduction cannot live
in the same frame; the pre-flight + deferred pattern is how SCI reproduces it.

**D-gas: gas is metered into the limit.** When `fee_payer == root`, gas consumption is
charged against root's `address(0)` sentinel limit (shared with native value) as
`gas_used × max_fee`. This matters because a reverting sponsored batch still burns
root's real ETH for gas — without the sentinel, a session key could drain root via
deliberately-reverting batches without ever touching a token limit.

### fee_payer gas sponsorship

revm's inner `validate_against_state_and_deduct_caller` always charges `tx.caller`
(the signer) and bumps its nonce. For sponsored gas the signer must net zero and the
`fee_payer` must pay. `SciHandler` therefore:

1. pre-funds the signer from `fee_payer` with the maximum the inner deduct can require
   (`gas_limit × max_fee` + the L1 data/operator `additional_cost`),
2. runs the inner deduct,
3. reconciles bidirectionally (return excess, or cover shortfall) so the signer nets
   exactly zero,
4. moves the unused-gas refund to `fee_payer` in `reimburse_caller`.

Mirroring the inner's `L1BlockInfo` fetch is required to make `tx_cost_with_tx` yield
the same `additional_cost` the inner will charge, keeping the reconcile delta to the
unused-gas remainder only. Sponsored gas is restricted to `fee_payer == root`: the
session key's right to spend root's funds is authorized by the keychain, whereas an
arbitrary third-party sponsor would need its own signature, which the AA tx does not
carry.

### Batch execution

`execute_aa_batch` executes each `Call` as its own depth-0 frame (mirroring revm's
`create_init_frame` but with `caller` overridden to `root`), wraps the whole batch in a
single journal checkpoint for atomicity, threads gas across calls, and normalizes the
final frame's gas to the full transaction gas limit. The tracing path mirrors this via
`inspect_run_exec_loop` so `debug_trace*` covers every call.

---

## 6. Plan A vs Plan B

The agent transaction carrier went through two designs:

| | Plan B (superseded) | Plan A (current) |
|---|---|---|
| Carrier | standard EIP-1559 tx + EIP-7702 delegation to a `SCIAgentDelegator` predeploy | native AA tx type `0x76` |
| Session key | hook reads `tx.from` | recovered from the keychain signature |
| Gas in limit | no (session key self-funds) | yes (`fee_payer` metered as D-gas) |
| Batching | via 7702 delegation + a forwarder contract | native `calls[]` |
| Change surface | precompile + thin handler | plus envelope / txpool / payload / derivation / proofs |

Plan A ports Tempo's AA execution model wholesale, gaining the protocol-level
guarantee that gas is metered into the limit and deductions roll back atomically. The
trade-off is that a native transaction type must be threaded through Base's shared
envelope and every consumer (consensus codec, rpc-types, execution receipt/pool
paths), and proven consistently by Kona / TEE / ZK — which is the largest engineering
and risk item.

---

## 7. Porting workflow

```
Tempo ships a new release (e.g. v1.7.2):
  1. preserve SCI-only sibling files (sci_ext.rs)
  2. cp the business files (account_keychain/{mod,dispatch}.rs, storage/*.rs,
     macros, ABI bindings)
  3. restore sci_ext.rs
  4. sed the alloy umbrella paths → individual alloy-* crates
  5. re-apply the Critical Rule #5 SCI patches (~15 stable edits)
  6. cargo check + 319 lib tests + 14 hook_e2e + 7 shim tests
```

The combination of Cargo `package =` renames and the `revm-shim` means the business
source is copied near-verbatim — identifier rewrites are no longer needed. Only the pinned
patch list needs re-applying, which keeps upstream sync tractable.

---

## 8. Known divergences and pitfalls

### Divergences vs Tempo (deliberate)

- **No TIP-20.** `is_tip20()` is stubbed to `true`; any ERC-20 transfer/approve target
  is token-like. The upstream recipient-constrained-scope test is `#[ignore]`d.
- **No Tempo handler execution path.** The hook is SCI's own
  `CircuitBreaker → Scope → SpendingLimit`, not a verbatim port of Tempo's
  `prevalidate_keychain_call_scopes` / `execute_multi_call`.
- **A circuit-breaker primitive Tempo lacks.** `SciAgentState` precompile + the hook's
  CB check are SCI-only.
- **Deferred deduction.** Because ERC-20 has no same-frame rollback, the pre-flight +
  deferred pattern replaces Tempo's TIP-20 in-frame deduction.

### Pitfalls found in review

- **M-1 — reverting batches drain sponsored gas.** A `fee_payer == root` batch that
  reverts still burns root's ETH, so the `address(0)` gas sentinel must be charged
  regardless of body outcome.
- **M-2 — `transferFrom(from == root)` is a spend.** The batch runs with
  `msg.sender == root`, so `transferFrom` where `from == root` spends root's funds and
  must count against the limit.
- **M-3 — zero-balance signer pool stuffing.** Sponsored AA transactions are
  keychain-authorized at pool admission (`fee_payer == root` structurally, plus a
  plausibly-active key record); the execution hook stays authoritative and the pool
  check fails open.
- **L-7 — missing `address(0)` sentinel.** An `enforce_limits` key with sponsored gas
  or native value must have an `address(0)` sentinel row; a missing row is treated as
  zero (reject), not unlimited.
