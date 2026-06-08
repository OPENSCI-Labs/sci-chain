# Plan A — L1 Forced-Inclusion / Censorship Escape Hatch (Design)

**Status:** design only — no code yet. Date: 2026-06-08.
**Branch:** `feat/plan-a-aa-keychain`.
**Driver:** item #8 of the decentralization remaining-work list. A centralized L2 sequencer
can censor transactions. For an agent permission-sandbox payments chain this is both a
safety problem (a compromised/rogue agent must be stoppable even if the sequencer censors
the stop) and a regulatory/credibility problem (the operator must not have unescapable
control over user funds/agents). OP-Stack's answer is **L1 forced inclusion via deposit
transactions**; this doc designs how that applies to SCI's keychain/AA model.

This is a companion to `plan-a-decentralization-gap.md` (which covers gossip / de-local-only).

---

## 1. Goal & threat model

The sequencer is honest-but-censoring (it may refuse to include specific txs) but cannot
forge state or drop L1-derived deposits. We want: **whatever a party could authorize on L2,
it can still get executed when censored, by forcing the action in from L1.**

Three distinct capabilities a censored party might need, in descending safety-criticality:

1. **Emergency freeze** — the root owner / guardian halts a misbehaving agent (trip the
   CircuitBreaker on a session key). *Most important: this is the kill-switch.*
2. **Key administration** — the root owner revokes a compromised session key, rotates keys,
   tightens limits.
3. **Delegated agent action** — execute the gated "agent acts as root" batch (the `0x76`
   semantics: scope + spending-limit + CB checks) without the sequencer's cooperation.
   *Least safety-critical: the agent is not the party we most need to protect; the root is.*

---

## 2. Background — how forced inclusion works here

Standard OP-Stack deposit-derivation pipeline (in-repo, not an external kona dep):

- **L1 entry:** a user calls `OptimismPortal.depositTransaction(...)` on L1, which emits
  `TransactionDeposited(address from, address to, uint256 version, bytes opaqueData)`
  (topic hash `0xb3813568…7c32`, `crates/consensus/protocol/src/deposits.rs:15,21-22`). The
  portal `.sol` is an **external L1 dependency**, not vendored here. Its address is
  `RollupConfig.deposit_contract_address` (`crates/common/genesis/src/rollup.rs:49-50`),
  injected at devnet bring-up from the op-deployer addresses JSON.
- **Derivation:** on the first L2 block of each L1 epoch, `derive_deposits` filters L1 receipts
  by the portal address (`crates/consensus/derive/src/attributes/stateful.rs:101,248-250`),
  decodes each into a `TxDeposit` (`deposits.rs:38-152`, `unmarshal_v0` `:221-267`), and injects
  them **after the L1-info tx, before pool txs, with `no_tx_pool: true`** (`stateful.rs:182-208`).
  The sequencer cannot drop them — this is the censorship-resistance guarantee.
- **Deposit shape:** `TxDeposit { source_hash, from, to, mint, value, gas_limit,
  is_system_transaction, input }` (`crates/common/consensus/src/transaction/deposit.rs:29-60`).
  `to` / `value` / `mint` / `gas_limit` / `input` all come from the L1 `opaqueData`
  (`abi.encodePacked(mint, value, gasLimit, isCreation, data)`), so the L1 caller controls an
  **arbitrary target + calldata + value**. Only **version 0** is decoded (`deposits.rs:142-146`).
- **Gas/funding:** deposits are L1-paid and effectively free on L2 — `mint` is credited before
  any deduction and the max-fee/funding check is skipped (`crates/common/evm/src/handler.rs:109-140`,
  mint at `:122-124`); no fee-vault payment (`:257-262`); even a failing deposit still bumps the
  nonce and credits `mint` (`:339-369`, `:356`). A zero-balance account can execute a deposit.

---

## 3. The four pivotal findings

1. **EOA L1 sender → L2 `msg.sender` = that EOA, UNCHANGED.** The L2 derivation does zero
   aliasing — it writes the portal's emitted `from` verbatim (`deposits.rs:54-67,135`; a
   repo-wide search for `applyL1ToL2Alias`/the alias offset found nothing in Rust). The
   OP-Stack-standard portal aliases **contract** senders (`from + 0x1111…1111`) but passes
   **EOAs** through (alias applied only when `msg.sender != tx.origin`). **So an L1 EOA at
   address `X` can force-include a call that executes on L2 with `msg.sender == X`.** (Must be
   confirmed against the actual deployed devnet portal — see §7.)
2. **Deposits (`0x7E`) do NOT seed the keychain `tx_origin` transient slot.** `SciHandler`
   early-returns for deposits at `crates/common/evm/src/sci_handler.rs:304-308`, *before* the
   `set_keychain_tx_origin` call at `:349` that every normal tx hits (`hook.rs:61-77`).
3. **The keychain admin gate requires `tx_origin`.** `ensure_account_caller`
   (`sci/crates/precompiles/src/account_keychain/mod.rs:1006-1029`) requires
   `transaction_key == 0` AND `tx_origin == msg_sender` (non-zero). Combined with (2): a
   deposit-triggered call to the keychain (e.g. `revokeKey`) **fails today** with
   `UnauthorizedCaller` (`tx_origin == 0`).
4. **CircuitBreaker authorizes on `msg.sender`, not `tx_origin`.** `AgentCircuitBreaker.trip`
   is `onlyGuardian` = `msg.sender == owner() || guardians[msg.sender]`
   (`sci/contracts/src/agent/AgentCircuitBreaker.sol:27-30,34-37`). It forwards to
   `SciAgentState.tripKey`, which only accepts `msg.sender == AGENT_CIRCUIT_BREAKER_ADDRESS`
   (`sci/crates/precompiles/src/sci_agent_state/mod.rs:39-50`) — satisfied because the façade
   is the caller. **So a deposit with `msg.sender == owner/guardian` can trip the breaker with
   no code change.**

---

## 4. Tiered design

| Tier | Capability | Status / what it needs |
|---|---|---|
| **1** | Emergency freeze: root/guardian force-includes `AgentCircuitBreaker.trip(sessionKey)` from L1 to halt a rogue agent | **DONE + VERIFIED on devnet 2026-06-08** (no SCI core code change). e2e `sci/devnet/e2e/l1-escape-hatch-cb.sh` PASSES: owner force-freeze → `isTripped=true` (~20s), owner force-unfreeze → `false`, and a **non-owner force-trip is rejected** (auth holds via L1). See §4.1. |
| **2** | Key admin: root force-includes `revokeKey` / `updateSpendingLimit` / etc. from L1 | Blocked by findings 2+3. Needs a **small, targeted handler change**: seed `tx_origin` for deposit txs too (§5). |
| **3** | Delegated agent action: force-include the gated `0x76` batch (scope + limit + CB) from L1 | HARD, `0x76`-hook-only (§6). Deferred. |

**Recommended order:** Tier 1 (verify + test), then Tier 2 (the handler change), then Tier 3
later as a separate project. Tiers 1+2 together give the root owner a censorship-resistant
**emergency-control** escape hatch (freeze + revoke), which is the safety/regulatory core.

### 4.1 Tier 1 — implemented & verified (2026-06-08)

No SCI core code change — Tier 1 is purely composing existing primitives + an e2e harness.

**Flow.** The emergency party (CB owner or a guardian) calls, on L1:
`OptimismPortal.depositTransaction(to=0xBBBB..03, value=0, gasLimit=250000, isCreation=false,
data=trip(sessionKey, reason))`. The deposit is force-derived into L2 (sequencer cannot drop
it) and executes with `msg.sender == the L1 EOA`; `AgentCircuitBreaker.trip` accepts it via
`onlyGuardian`, freezing the session key in `SciAgentState`. `untrip(sessionKey)` resumes it.

**Runbook / e2e:** `sci/devnet/e2e/l1-escape-hatch-cb.sh` (run on the devnet host). It discovers
the portal from `.devnet/l2/configs/l1-addresses.json`, then runs three checks: owner
force-freeze (→ `isTripped=true`), owner force-unfreeze (→ `false`), and a negative control
(non-owner force-trip → stays `false`). Verified PASS on devnet 2026-06-08 (each deposit
derived in ~20s; portal address `0x68b2f0ad…71869`, CB owner ACC0 `0xf39F…2266`).

**Note:** the keychain hook already enforces the tripped state at agent-tx time (a `0x76` tx
from a tripped session key is rejected; the trip→reject→untrip→include cycle was devnet-proven
earlier as Plan A "P4"). Tier 1's novelty is that the *trip itself* is now censorship-resistant
via L1, so the kill-switch cannot be withheld by the sequencer.

---

## 5. Tier 2 in detail — seed `tx_origin` for deposits

**Change:** in `SciHandler::validate_against_state_and_deduct_caller`, do not skip
`tx_origin` seeding for deposits. Today the deposit early-return (`sci_handler.rs:304-308`)
exists to bypass the `0x76` keychain **hook** (correct — deposits aren't agent txs). But it
also skips `set_keychain_tx_origin` (`:349`). Split the two: still skip the agent hook for
deposits, but seed `tx_origin = tx.caller()` (and `transaction_key = 0`) for deposits as well,
so a deposit-triggered call satisfies `ensure_account_caller`.

**Why this is safe (the authentication argument):**
- For an **EOA** L1 sender, the deposit `from` (= L2 `msg.sender` = the seeded `tx_origin`) is
  the EOA itself — and only the holder of that key could have produced the deposit. So seeding
  `tx_origin = from` lets exactly the authentic account administer *its own* keychain
  (`keys[msg_sender]`), which is precisely what a normal self-sent tx does. No impersonation:
  the keychain only ever mutates `keys[msg_sender]`.
- For a **contract** L1 sender, `from` is aliased (`addr + 0x1111…1111`), which is a distinct
  address that owns its own (empty) keychain — it cannot touch the real account's keychain.
- **System deposits** (L1-info tx, upgrade txs) never call the keychain, so seeding `tx_origin`
  for them is inert.
- `transaction_key` stays `0` for deposits (no agent hook runs), so the spending-limit metering
  hooks remain no-ops (`mod.rs` `authorize_transfer`/`authorize_approve` no-op when
  `transaction_key == 0`). Deposits get no agent-delegation powers — only self-admin.

**Caveat:** this touches the consensus-critical handler and the OP-Stack deposit path; it must
be verified to not perturb system-deposit ticks (L1-info / predeploy upgrades) and to be
identical across the sequencer, verifier, and the ZK/TEE proof program. Needs a unit test + a
devnet e2e (force a `revokeKey` deposit from the root EOA, confirm the key is revoked).

---

## 6. Tier 3 in detail — delegated agent action (deferred)

The gated "agent acts as root" semantics (keychain authorization `keys[root][session]`,
per-call scope, spending-limit pre-flight/deduction, sponsored gas) live **exclusively in the
Rust `0x76` pre-execution hook** (`run_aa_keychain_hook`); there is **no reusable callable
authorization entrypoint** (the predicates `key_is_active`, `validate_call_scope_for_transaction`,
`effective_remaining_limit` are internal, hook-only — `sci/crates/precompiles/src/account_keychain/{mod,sci_ext}.rs`).
A deposit (`0x7E`) skips the hook entirely. Options, in rough effort order:

- **Option A (low-med) — deposit-as-trigger relay.** Force-include a deposit whose `to` is an
  L2 relay predeploy that performs the action and re-implements the keychain checks in Solidity
  by calling keychain *view* functions. Caveat: `msg.sender` is the deposit `from` (the L1 EOA),
  not a session key, so this is really "root acts directly" — which Tier 2 already covers for
  self-admin. Re-implementing scope/limit in Solidity is duplicate, drift-prone logic.
- **Option B (high) — new deposit version embedding a signed `0x76`.** Extend
  `Deposits::unmarshal` with a `v1` that decodes an embedded `BaseAaTransaction` and emits an
  `OpTxType::Aa` from derivation. Changes the consensus-critical derivation format and ripples
  into fault proofs (`crates/proof/`). Not recommended without strong need.
- **Option C (med-high) — L2 forced-inclusion inbox predeploy.** A deposit stores a signed
  `0x76` blob in an inbox; a separate mechanism replays it through the real hook. Decouples L1
  auth from session-key auth but still needs the replay to route through the `0x76` path.

**Assessment:** Tier 3 is genuinely hard and arguably unnecessary — the censored party that
matters (the root owner) is fully served by Tiers 1+2 acting *as root directly*. An agent
being unable to force its own payment through is an acceptable limitation. Revisit only if a
concrete requirement for trustless agent-action forced inclusion emerges.

---

## 7. Open questions / to verify before implementing

1. **Portal aliasing on the actual devnet.** ✅ RESOLVED 2026-06-08 — a live deposit from the
   ACC0 EOA executed on L2 with `msg.sender == ACC0` (the `onlyGuardian` trip succeeded), so the
   deployed portal passes EOAs through unaliased as assumed.
2. **Who is CB owner/guardian on devnet?** ✅ RESOLVED — `AgentCircuitBreaker.owner()` is ACC0
   (`0xf39F…2266`). The emergency party on this devnet is ACC0; other parties can be added via
   `setGuardian` (onlyOwner). Production deployments must set the owner/guardian to the intended
   emergency-control party.
3. **Portal minimum gas-limit floor** (lives in the un-vendored `.sol`) — a `trip` deposit at
   `gasLimit=250000` is accepted (verified). Re-check the floor for heavier Tier 2 calls
   (`revokeKey`, etc.).
4. **Proof-program parity** for the Tier 2 handler change — the ZK/TEE program must seed
   `tx_origin` for deposits identically to the sequencer, or state roots diverge. (Tier 2, not
   yet implemented.)

---

## 8. Non-goals

- This does not decentralize the sequencer (that follows Base upstream — separate item).
- Tier 3 (trustless forced agent action) is explicitly out of scope for now.
- No change to the happy-path `0x76` flow.

---

## 9. Decision

2026-06-08: produced this design doc, then implemented & verified **Tier 1** on devnet (e2e
`sci/devnet/e2e/l1-escape-hatch-cb.sh`, no core code change — §4.1). Open questions §7.1/§7.2
resolved. **Tier 2** (seed `tx_origin` for deposits → L1-forced key admin) and **Tier 3**
(trustless agent-action forced inclusion) remain; recommended next is Tier 2.
