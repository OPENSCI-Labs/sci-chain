# Decision Memo: Agent On-Chain Registration Path (ERC-8004 + ERC-6551 vs. Plan A keychain)

> **Update (2026-06-05):** the `SciAgentRegistrar` contract has been **removed** from this
> branch along with the rest of the Plan B layer. Its `registerAgent` one-step helper only
> worked under EIP-7702 (it authorizes via `msg.sender`), which Plan A does not use. Option B
> registration is therefore implemented directly: the root account calls
> `keychain.authorizeKey` and (optionally) `registry.bindKey` itself — exactly the Phase 6
> e2e flow in `plan-a-aa-e2e.md`. The comparison below is retained as the original decision
> rationale; read "`SciAgentRegistrar`" in it as "the one-step registration shape", now done
> by direct calls rather than a helper contract.

**Status:** DECIDED 2026-06-04 — **Option B** for v1 (see "Decision" below).
**Date:** 2026-06-04.
**Context:** Product feedback (2026-06-03): the agent on-chain registration path
`ERC-8004 (agent identity) → ERC-721 (IDA NFT) → ERC-6551 (token-bound account)`
feels too long. This memo compares three paths so we can pick one before the
Phase 6 agent-loop e2e (the e2e must first fix what an agent's "root account" is).

## TL;DR

The feedback is **not** about §6.3 (that is an execution-test rewrite, 7702→AA). It is
about the **identity/account-provisioning layer**. The key technical fact: **Plan A's
keychain + native AA tx (`0x76`) already provide account abstraction for execution**, so
an ERC-6551 token-bound account (TBA) is **redundant for the "agent can act on-chain"
goal** — 6551's only remaining value is *identity/ownership composability*. Recommendation:
**Option C** (keep an identity NFT, drop the 6551 TBA from the execution path), with
**Option B** as the minimal fallback if an on-chain identity NFT is not required for v1.

## Current code state (grounded)

- `SciAgentRegistrar.registerAgent(keyId, sigType, restrictions, agentId)` is **already a
  one-step (ERC-8004-inspired) call**: it runs `keychain.authorizeKey` + `registry.bindKey`
  in **one tx**, and emits `IDAMintRequested` as a stub.
- The **IDA NFT (ERC-721 + ERC-6551 TBA) contract does not exist yet** in `sci/contracts`
  (only the solady lib ships 6551 helpers). So nothing is locked in — full design freedom.
- The keychain indexes `keys[root][sessionKey]` by `msg.sender`; an AA tx is signed by the
  session key and names `root`, and the handler runs the batch with `msg.sender == root`.
  `root` can be **any** account — a plain EOA, a gateway-provisioned account, or a contract
  (a 6551 TBA) — the mechanism does not care.

## The three options

### A. Full path — ERC-8004 + ERC-721 IDA + ERC-6551 TBA (status-quo design)
Agent identity in an 8004 registry; IDA minted as an ERC-721; a 6551 TBA deployed as the
agent's operable account; the TBA then authorizes a keychain session key. The AA tx's
`root` = the TBA.

### B. Slim path — ERC-8004 one-step + keychain only (no IDA NFT, no TBA)
`registerAgent` authorizes the session key over a plain root account and binds an
off-chain `agentId` (DID / 8004 record). No on-chain identity NFT, no TBA. The AA tx's
`root` = the plain account. **This is essentially what is already implemented.**

### C. Hybrid — ERC-721 IDA for identity, keychain root for execution (no TBA)
Mint an ERC-721 IDA NFT purely as an identity/ownership credential (and optionally an
8004 record for discovery), but execution uses a keychain-governed root account, **not** a
6551 TBA. The NFT can *reference* the root account (and vice-versa) without being its
account. The AA tx's `root` = the keychain root account.

## Comparison

| | A. Full (8004+721+6551) | B. Slim (8004+keychain) | C. Hybrid (721 identity + keychain) |
|---|---|---|---|
| On-chain txs to first agent action | ~4–5 (8004 register, mint NFT, createAccount TBA, TBA→authorizeKey, bind) | **1** (`registerAgent`) | ~2 (mint IDA NFT, `registerAgent`) |
| New contracts to build | IDA ERC-721 + 6551 registry/account impl + registrar wiring (mint + createAccount) | none (registrar exists; IDA stub → off-chain) | IDA ERC-721 (identity only) + thin registrar wiring |
| Contract change scope | **Large** | **~zero** | **Medium** |
| "Root account" in AA tx | 6551 TBA (a contract) | plain account (EOA/gateway acct) | keychain root account (EOA/gateway acct) |
| On-chain identity artifact | NFT + TBA | none (off-chain agentId only) | NFT (identity only) |
| Redundancy | **Yes** — 6551 TBA *and* keychain both provide account abstraction for execution | none | none (NFT is identity, not an account) |
| Phase 6 e2e impact | Heaviest: must provision a TBA and drive `authorizeKey`/AA tx with `root = TBA` (contract caller); aa-txgen `ROOT` = TBA addr | Lightest: `authorizeKey` from a root EOA + AA tx — **matches the current devnet repro** (root = ACC1) | Light: identity-NFT mint is off the hot path; execution e2e identical to B |
| Audit surface | Largest (NFT + 6551 + TBA exec) | Smallest | Medium |

## Key technical insight

Plan A already replaces the *execution* reason for a 6551 TBA: the keychain + AA tx are the
account-abstraction layer. Stacking a 6551 TBA on top (Option A) means **two** AA
mechanisms for one job — which is the length the product is reacting to. The only thing
6551 still buys is treating the agent account as a transferable NFT-owned object; if that
ownership/composability is wanted, Option C keeps the NFT for *identity* while letting the
keychain own *execution*.

## Recommendation

- **Option C** if an on-chain agent identity/ownership NFT is a product requirement
  (discovery, transferability, ERC-8004 interop): keep the IDA ERC-721, drop the 6551 TBA
  from the execution path. ~2 steps, medium build, no redundancy.
- **Option B** if a v1 agent only needs to *act* (no on-chain identity NFT yet): ship what
  exists (1 step), defer the IDA NFT. Easiest to reach a full Phase 6 e2e now.
- **Avoid Option A** unless 6551-TBA-as-the-agent-account is an explicit external
  requirement — it adds the most steps, contracts, and audit surface for capability the
  keychain already provides.

## What this changes downstream

- **§6.3 / Phase 6 e2e:** B and C both make `root` a plain keychain account, so the AA-flow
  e2e is exactly today's devnet repro (`authorizeKey` from root → `sci-aa-txgen` AA tx →
  limit/CB/expiry). A would require provisioning a TBA and using it as `root`.
- **`SciAgentRegistrar`:** B/C keep the current one-step shape; C adds an IDA-NFT mint call
  (replacing the current `IDAMintRequested` stub); A adds NFT mint + 6551 `createAccount`.

## Decision (2026-06-04): Option B for v1

We ship **Option B** now: ERC-8004-style one-step registration (`SciAgentRegistrar`,
already implemented) + keychain only. No on-chain IDA NFT, no ERC-6551 TBA. The agent's
operable account is a plain keychain root account; `agentId` is an off-chain identifier
(DID / ERC-8004 record resolved by the gateway). Agent identity is **not transferable** in
v1 — operator changes are handled by keychain key rotation (`authorizeKey`/`revokeKey`),
which is a different concern from ownership transfer.

Rationale (from the discussion):
- Most v1 agents are **operated services**, not tradable assets. "Change who operates it" is
  key rotation (keychain already does this), not identity transfer.
- An agent's actual capability (model/weights/prompts/tools) lives **off-chain**, so an
  on-chain ownership transfer does not, by itself, make a buyer able to run the agent — that
  requires an off-chain capability delivery/service model we are not building in v1.
- Transferable on-chain reputation also invites reputation-laundering; non-transferable is
  often preferable anyway.

Revisit trigger → upgrade to **Option C** (add a soulbound-or-transferable identity NFT;
keep keychain for execution) **if** a product requirement emerges for a transferable agent
*instance* (marketplace / custody change carrying reputation + wallet), paired with a
defined off-chain capability handoff. Option C is an additive layer over B — no rework of
the B execution path.

Implementation under B: `SciAgentRegistrar` drops the `IDAMintRequested` stub (no IDA
contract in v1); `agentId` documented as off-chain. Note the `SciAgentRegistrar.registerAgent`
one-step helper only works under EIP-7702 delegation (it authorizes via `msg.sender`); under
Option B (no 7702) registration is the **root account directly calling `keychain.authorizeKey`**
(+ optional `registry.bindKey`). The Phase 6 agent-loop e2e uses a plain keychain root account
(= today's devnet AA repro) — see `plan-a-aa-e2e.md`.

## Open questions for sign-off

1. Is an **on-chain, transferable agent-identity NFT** a v1 requirement, or is an off-chain
   `agentId`/DID enough? (B vs. C.)
2. Is **ERC-6551-TBA-as-the-agent-account** required by any external integration (e.g. a
   partner expecting the agent to *be* a token-bound account)? If not, drop it.
3. Should `agentId` map to an **ERC-8004 registry record** on SCI Chain, or stay an opaque
   identifier resolved by the gateway?
