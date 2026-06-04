# Plan A — Consensus-Critical-Path Audit Scope

**Status:** Draft for reviewers. **Date:** 2026-06-04. **Branch:** `feat/plan-a-aa-keychain`.

Scope of the audit: the native AA transaction type (`0x76`, `BaseAaTransaction`) and the
keychain pre-execution hook that gates it. These are the only consensus-critical additions
of Plan A — everything else (the keychain precompile internals, shim crates) is either a
verbatim Tempo port or non-consensus. This note lists the files, the invariants reviewers
should confirm, the known intentional divergences, and the evidence gathered so far.

## 1. Transaction type: decode / encode / signing

**Files:** `crates/common/consensus/src/transaction/aa.rs` (core type + RLP/2718 codec +
`Transaction`/`Typed2718`/`SignableTransaction` impls), `transaction/{envelope,typed,tx_type}.rs`
(variant wiring), `pooled.rs`, `core.rs` (`FromTxWithEncoded` execution arm),
`reth_compat.rs` (reth `Compact` + serde-bincode-compat).

**Invariants to confirm:**
- **Signature covers the right fields.** `BaseAaTransaction` is signed by the **session key**
  with a standard secp256k1 signature over `{chain_id, nonce, fees, gas_limit, calls[],
  access_list, fee_payer, root}`. Confirm the signing payload includes `calls[]`, `fee_payer`
  AND `root` — so none of them can be tampered post-signing.
- **`root` is a *claimed* field, not proven by the signature's recovered address.** The signer
  is the session key; `root` is whoever the calls execute as. The ONLY thing preventing
  `root = victim` impersonation is the keychain gate (§3). Confirm there is no execution path
  that honors `root` without the gate.
- **Codec round-trips** (RLP, 2718, reth Compact, bincode) are lossless and type-tagged at
  `0x76`; no variant-enumeration site silently treats AA as another type.

## 2. Fee / gas deduction (sponsored gas)

**Files:** `crates/common/evm/src/sci_handler.rs::validate_against_state_and_deduct_caller`
(fee_payer pre-fund + bidirectional reconcile), `::reimburse_caller`, `::execution_result`
(deferred keychain deduction); `core.rs` (AA base `TxEnv` value=0).

**Invariants:**
- **Conservation / no mint.** When `fee_payer == root` sponsors: the signer nets exactly 0
  (balance unchanged, nonce +1), the fee_payer bears gas + L1 data cost + native value, the
  recipient receives value. No wei is created or destroyed. (Devnet-verified, see §6.)
- **Pre-fund covers the full inner-deduct requirement.** The pre-fund must equal
  `max_balance_spending() + tx_cost_with_tx` (L2 gas + L1/operator), or a 0-balance signer
  underflows `ensure_enough_balance` before the reconcile runs (this was follow-up #5; fixed
  in commit 177f7d523). Confirm the L1BlockInfo fetch is mirrored so the inner reuses the same
  `additional_cost`.
- **`fee_payer == root` enforced.** The handler rejects `fee_payer != root` (an arbitrary
  third-party sponsor would need its own signature the AA tx does not carry). Confirm no bypass.
- **Refund routing.** Unused-gas + operator-fee refund is moved to `fee_payer` in
  `reimburse_caller`. Confirm it can't be redirected.

## 3. Keychain authorization gate (the security boundary)

**Files:** `crates/common/evm/src/sci_handler.rs` (builds the `AaCall` batch, calls the hook);
`sci/crates/precompiles/src/handler/{mod,hook,decode}.rs` (`run_aa_keychain_hook`);
`sci/crates/precompiles/src/account_keychain/*` (keychain state, Tempo port);
`sci/crates/precompiles/src/sci_agent_state/` (circuit-breaker trip state).

**Invariants (highest priority — this is what makes `root` impersonation safe):**
- **Authorization.** For a `root=Some` AA tx, `keys[root][session_key]` must be **active**
  (exists, not revoked, not expired) or the whole tx is rejected pre-execution. Confirm there
  is no path where `root` is honored without this check (the 2a/2b paths were ungated before
  2c-i — commit 46cc3fd24 added the gate; confirm it is unconditional).
- **Circuit breaker.** A tripped session key's batch is rejected before execution. Trip state
  lives in `SciAgentState` (`0xAAAA..01`), mutated only by `AgentCircuitBreaker` (`0xBBBB..03`).
- **Per-call scope.** Each `Call` is checked against the key's `allowedCalls` scope when not
  `allowAnyCalls`.
- **Checkpoint isolation.** The hook wraps its transient writes (`transaction_key`, `tx_origin`)
  in a journal checkpoint and reverts on failure — a hook reject leaks no partial state.

## 4. Atomic batch execution + revert

**Files:** `sci_handler.rs::execution` / `::execute_aa_batch` / `::inspect_execution`,
`::finalize_batch_gas`.

**Invariants:**
- **All-or-nothing.** Any single call's failure reverts the entire batch under one outer
  journal checkpoint; no partial side effects persist. (Devnet-verified, follow-up #3.)
- **Per-call `msg.sender == root`.** Each call runs as a depth-0 frame with caller = `root`.
- **Gas normalization.** `finalize_batch_gas` marks the full `gas_limit` spent then credits
  remaining, matching revm's single-call `last_frame_result` semantics.
- **Inspector parity.** The tracing path (`inspect_execution`) runs the same batch loop, so
  `debug_trace*` covers all calls (follow-up #2), not just the first.

## 5. Spending-limit metering (deferred, strong-R1)

**Files:** `run_aa_keychain_hook` (pre-flight), `apply_aa_post_execution_deductions`,
`sci_handler.rs::execution_result`.

**Invariants:**
- **Read-only pre-flight, deferred write.** The hook only *checks* the batch fits remaining
  quota; the real deduction is applied in `execution_result` **only when the body succeeded**.
  Hook reject / body revert / OOG → no deduction (agent keeps quota). (Verified by hook_e2e
  `body_revert_rolls_back_deduction_strong_r1` + devnet follow-up #4.)
- **Sentinel accounting (D2-B / D-gas).** Native value + gas (`gas_used * max_fee`) meter into
  the `address(0)` sentinel limit; ERC-20 (incl. SCI-20 `transferWithMemo`) meter per-token.
  Confirm gas reservation = `gas_limit * max_fee` (pessimistic) at validate, deduction
  = `gas_used * max_fee` at result.
- **Pessimistic.** `approve` counts as a full max-commitment (no refund of unused allowance).

## 6. Txpool admission (local-only, fundless sponsor)

**Files:** `crates/execution/txpool/src/{validator,transaction}.rs`,
`crates/execution/node/src/node.rs` (`with_custom_tx_type(0x76)`),
`crates/common/consensus/.../envelope.rs::as_aa`.

**Invariants:**
- **Never gossiped.** AA txs are local-only (`propagate = false`); confirm no path converts an
  AA tx to an alloy `PooledTransaction`/`TxEnvelope` for network egress.
- **Sponsor-aware balance check.** Admission checks the **fee_payer/root** balance (not the
  0-balance signer) when sponsored; the pooled `cost` override (gas only if signer pays gas,
  value only if root pays value) keeps admission and per-block maintenance consistent
  (follow-up #1). nonce still checked against the signer.

## Known intentional divergences (NOT bugs — confirm acceptable)

- AA receipts map to an EIP-1559 receipt; RPC `TransactionRequest` surfaces a first-call
  EIP-1559 approximation (PoC simplifications; tracked for backfill).
- `is_tip20(target)` is stubbed to `true` — recipient/selector restrictions apply to any
  transfer/approve target (SCI uses standard ERC-20, no TIP-20 factory).
- `SCIAgentDelegator` (`0xCCCC..01`) is a Plan B / EIP-7702 legacy compat contract, NOT on the
  Plan A hot path; agent registration under Option B is the root directly calling
  `keychain.authorizeKey` (see `agent-registration-path-decision.md`).

## Evidence gathered

- Phase 1 (tx type) + Phase 2 (2a multi-call / 2b fee_payer / 2c keychain + limit) +
  follow-ups #1–#5 devnet-verified; SP1 zkVM **execute** proves AA + keychain blocks are
  provable with keychain adding ~0.045% of cycles (derivation-dominated).
- Phase 6 cohesive Option-B loop on devnet: register → AA transfer (signer net 0) → circuit
  breaker (trip rejects / untrip re-includes the same tx) verified; spending-limit pass/reject
  verified (follow-up #4 + Phase 6).

## Operational caveat for reviewers running the devnet e2e

The devnet sequencer can stall in `AwaitingSafeHeadConfirmation` when the unsafe head drifts
too far ahead of the safe head (batcher/L1-derivation lag). Restarting the CL nodes
(`base-{builder,client}-cl`) mid-run can itself trigger unsafe-chain **reorgs** that reset the
batcher pipeline — prefer letting the chain settle over repeated restarts. This is a devnet
ops/config characteristic, **not** a Plan A consensus defect (the AA/keychain logic is
independent of L1 batching), but it affects e2e reproducibility.
