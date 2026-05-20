---
title: "P0-2 Contracts Integration Guide — Testing Solidity Contracts Against the Keychain Precompile"
audience: "Engineer landing the P0-2 Solidity contracts (Heath)"
prerequisite_branch: "feat/p0-1-keychain (merged or used as a base)"
date: "2026-05-21"
---

# P0-2 Contracts Integration Guide

This document explains how to land and test the P0-2 Solidity contracts
(`AgentAccessKeyRegistry`, `AgentBudgetController`, `AgentCircuitBreaker`,
`SCIAgentDelegator`, IDA ERC-721 + ERC-6551 TBA) against the keychain
precompile that `feat/p0-1-keychain` already ships. It assumes you are the
contracts engineer ("S" lane in CLAUDE.md `## Branches`) working on a
sibling branch `feat/p0-2-contracts`.

## What You Get From `feat/p0-1-keychain`

After this branch is merged or pulled in, the following are live on any
devnet running the `:sci` Docker image:

| Address | Component | Status on `:sci` devnet |
|---|---|---|
| `0xAAAAAAAA00000000000000000000000000000000` | `AccountKeychain` (Rust precompile) | **Live.** Has `code: "0xef"` in genesis; state writes persist; all ABI functions in `IAccountKeychain.sol` route through the precompile. |
| `0xAAAAAAAA00000000000000000000000000000001` | `SciAgentState` (Rust precompile, CB flag store) | **Live.** Same setup as above; only the `0xBBBB...03` address may write via `tripKey` / `untripKey`. |
| `0xBBBBBBBB00000000000000000000000000000001` | `AgentAccessKeyRegistry` (your contract) | **Empty.** You deploy. |
| `0xBBBBBBBB00000000000000000000000000000002` | `AgentBudgetController` (your contract) | **Empty.** You deploy. |
| `0xBBBBBBBB00000000000000000000000000000003` | `AgentCircuitBreaker` (your contract) | **Empty.** You deploy. |
| `0xCCCCCCCC00000000000000000000000000000001` | `SCIAgentDelegator` (your contract, EIP-7702 target) | **Empty.** You deploy or pre-allocate. |

The Rust pre-execution hook (`SciHandler`) is also live: every transaction
on the `:sci` devnet flows through it. The hook is a no-op for any tx
whose `tx.to` is not 7702-delegated to `SCI_AGENT_DELEGATOR_ADDRESS`, so
normal traffic and your contract deployments are unaffected. The hook only
activates the keychain checks once you wire EIP-7702 delegation to your
`SCIAgentDelegator`.

The keychain ABI you'll need is at
`sci/crates/precompile-abi/src/precompiles/account_keychain.rs`
(Rust + alloy `sol!` macro) and the corresponding Solidity interface
should live under `sci/contracts/src/interfaces/IAccountKeychain.sol`
(your scope).

## Three Testing Paths

### Path A — Shared Remote Devnet (recommended for early iteration)

A devnet built from `feat/p0-1-keychain` is already running on a shared
host. Point your tooling at it directly; nothing to install locally beyond
`foundry` / `forge`.

```bash
# Connection info
export L2_RPC=http://54.255.70.252:8545
export CHAIN_ID=42001

# Test accounts (Anvil mnemonic — both have 10,000 ETH on L2)
# Reserve account 0 for the "primary engineer" lane (P0-1).
# Use account 1 for your contracts work so we don't clash on nonces.
export DEPLOYER_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
export DEPLOYER_ADDR=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
```

Deploy and test:

```bash
cd sci/contracts
forge create \
  --rpc-url $L2_RPC \
  --private-key $DEPLOYER_PK \
  src/agent/AgentAccessKeyRegistry.sol:AgentAccessKeyRegistry

# Call into the keychain precompile from a test
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)((uint8,address,uint64,bool,bool))' \
  $DEPLOYER_ADDR \
  0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC \
  --rpc-url $L2_RPC
```

**Best for**: validating contract logic and its interaction with the
keychain precompile. Your contracts deploy to whatever addresses forge
hands out — no need for the fixed `0xBBBB...` / `0xCCCC...` predeploy
addresses yet.

**Constraints**:
- Shared host — coordinate nonces. Reserve test account 0 for P0-1, use
  account 1 (above) for P0-2.
- Chain state persists across our work and yours. If you need a clean
  slate, ping the P0-1 owner before requesting a `just devnet down`.

### Path B — Pre-allocate Your Contracts to the Fixed Predeploy Addresses

Some tests require your contracts at the exact addresses
`0xBBBB...01/02/03` and `0xCCCC...01`. That means adding genesis allocs
the same way the keychain precompile got `code: "0xef"`. Procedure:

1. **You**: produce `deployedBytecode` (runtime code, *not* init bytecode)
   for each contract via `forge inspect`:

   ```bash
   forge inspect AgentAccessKeyRegistry deployedBytecode
   forge inspect AgentBudgetController  deployedBytecode
   forge inspect AgentCircuitBreaker    deployedBytecode
   forge inspect SCIAgentDelegator      deployedBytecode
   ```

2. **You**: if the contract has constructor state to set (e.g. `owner`
   immutable, initial admin, etc.), enumerate the **initial storage
   slots** you need pre-populated. The genesis-alloc form is:

   ```json
   {
     "0xbbbbbbbb00000000000000000000000000000001": {
       "nonce": "0x0",
       "balance": "0x0",
       "code":   "0x<deployedBytecode>",
       "storage": {
         "0x0000000000000000000000000000000000000000000000000000000000000000": "0x000000000000000000000000<admin-address>",
         "...": "..."
       }
     }
   }
   ```

   `forge inspect <Contract> storageLayout` helps map fields to slots.

3. **Hand the JSON fragment to the P0-1 owner.** They merge it into
   `sci/devnet/sci-allocs.json` and re-bring up the devnet. The bring-up
   flow is documented in
   `sci/docs/feat-p0-1-keychain-branch-summary.md` §4.

4. After the devnet restart, `cast code 0xBBBB...01` returns your
   `deployedBytecode`, and the contract can be called as if it had been
   deployed at that address.

**For `SCIAgentDelegator` (`0xCCCC...01`) you have two options**:

- **Option B1 — Genesis pre-deploy** (same procedure above). Simple but
  static: any change to the delegator requires a devnet restart.
- **Option B2 — Deploy + EIP-7702 set-code at runtime**. Deploy
  `SCIAgentDelegator` to a regular address, then have each root EOA emit
  an EIP-7702 `setCode` authorization pointing at it. Closer to the
  production flow (real agent UX); requires more orchestration in tests.
  The pre-execution hook detects the 7702 header on `tx.to` and routes
  via `delegated_address == SCI_AGENT_DELEGATOR_ADDRESS`, so both options
  work — pick whichever fits your test ergonomics.

**Best for**: T8 end-to-end agent-transaction tests, and any test that
asserts the exact `0xBBBB.../0xCCCC...` address values.

**Coordination cost**: each contract ABI / storage layout change requires
a fresh genesis alloc and a devnet restart (chain state is wiped). Freeze
the contract interfaces before requesting a Path B run.

### Path C — Your Own Local Devnet

For heavy contract iteration where you don't want to coordinate with a
shared host, run the whole stack on your machine.

```bash
git clone https://github.com/OPENSCI-Labs/sci-chain.git
cd sci-chain
git checkout feat/p0-1-keychain         # has the keychain + sci/devnet/ patches
# Follow sci/docs/feat-p0-1-keychain-branch-summary.md for the full bring-up flow.
# Key steps (also in the branch summary):
#   1. Build base-only release images from ~/sci-dev/base-v0.8/ (clone of pure base)
#   2. Build SCI release images from this repo, tag as :sci
#   3. just devnet down; bring up L1 stack; bring up setup-l2 only
#   4. Apply sci/devnet/apply-sci-allocs.sh to .devnet/l2/configs/genesis.json
#   5. Patch rollup.json + rollup-conductor.json with the new genesis hash
#   6. Bring up base-client + base-builder with the sci compose override
```

**Best for**: rapid contract-side iteration; lets you do `just devnet down`
freely without coordinating.

**Setup cost**: docker buildx + Rust 1.93.1 + foundry; cold build is ~30
minutes for the SCI release images.

## Recommended Timeline

| Phase | Path | What you do |
|---|---|---|
| Now → contract API stabilises | **Path A** | Validate contract logic end-to-end against the live keychain precompile. No address constraints. Iterate freely. |
| Contract API frozen | **Path B** | Produce `deployedBytecode` + initial storage for each predeploy. P0-1 owner merges into `sci/devnet/sci-allocs.json`, restarts devnet. T8 unblocks. |
| Pre-merge to `main` | **PR integration** | Rebase `feat/p0-2-contracts` on top of `feat/p0-1-keychain` (or merge `p0-1` into your branch). Both lanes ship as a coherent PR. |

## What the Pre-Execution Hook Expects From Your `SCIAgentDelegator`

For T8 (full agent-tx loop) to work, the hook on the Rust side decodes the
outer transaction's calldata as:

```solidity
function execute(Call[] calldata calls) external;

struct Call {
    address target;
    uint256 value;
    bytes   data;
}
```

The ABI lives at
`sci/crates/precompile-abi/src/predeploys/sci_agent_delegator.rs`. Please
keep your Solidity `SCIAgentDelegator.execute(Call[])` signature
**bit-for-bit identical** to this — the hook's `decode_execute_batch`
will silently fall back to single-call mode if the ABI drifts, which
breaks batched-tx tests.

The `SCIAgentDelegator` must also enforce:

```solidity
require(getTransactionKey() != address(0), "no session key");
```

where `getTransactionKey()` calls the keychain precompile. The
pre-execution hook seeds the keychain's transient `transaction_key` slot
to the session key when (and only when) it has verified the tx is a
proper agent tx. So this check is what makes "session-key authorization"
load-bearing — without it, anyone could call `execute` directly without
hook gating.

Reference: `CLAUDE.md` → "Pre-execution Hook Design (P0-1.7 / P0-1.8)" →
"Agent-tx identification (Q1)" has the full rationale.

## What the Pre-Execution Hook Expects From `AgentCircuitBreaker`

`AgentCircuitBreaker` at `0xBBBB...03` is a Solidity façade in front of
the Rust `SciAgentState` precompile at `0xAAAA...0001`. The contract owns
admin access control (only allowed admins can trip / untrip), emits
events, and forwards to the precompile:

```solidity
function tripKey(address sessionKey) external onlyAdmin {
    ISciAgentState(0xAAAAAAAA00000000000000000000000000000001).tripKey(sessionKey);
    emit AgentTripped(sessionKey, msg.sender, block.timestamp);
}
```

`SciAgentState.tripKey` rejects any caller other than
`AGENT_CIRCUIT_BREAKER_ADDRESS` (`0xBBBB...03`). That check is enforced
in the Rust precompile, so the contract façade is the *only* way to trip
a key. CLAUDE.md → "CircuitBreaker state location (Q3)" has the rationale
for splitting state into Rust and admin into Solidity.

## Useful Commands Cheat Sheet

```bash
# Devnet health
cast chain-id      --rpc-url $L2_RPC                   # expect 42001
cast block-number  --rpc-url $L2_RPC                   # should be growing
cast code 0xAAAAAAAA00000000000000000000000000000000 --rpc-url $L2_RPC   # expect 0xef

# Query the keychain (read)
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)((uint8,address,uint64,bool,bool))' \
  $ROOT_ADDR $SESSION_KEY --rpc-url $L2_RPC

# Authorize a session key (write, signed by the root EOA)
cast send 0xAAAAAAAA00000000000000000000000000000000 \
  'authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))' \
  $SESSION_KEY 0 '(18446744073709551615,false,[],true,[])' \
  --rpc-url $L2_RPC --private-key $ROOT_PK

# Check trip status
cast call 0xAAAAAAAA00000000000000000000000000000001 \
  'isTripped(address)(bool)' $SESSION_KEY --rpc-url $L2_RPC

# Sign and broadcast an EIP-7702 set-code authorization
AUTH=$(cast wallet sign-auth $SCI_AGENT_DELEGATOR_ADDR \
  --private-key $ROOT_PK --rpc-url $L2_RPC)
cast send --rpc-url $L2_RPC --private-key $ROOT_PK --auth $AUTH \
  $ROOT_ADDR 0x   # any payload — the auth is what matters
```

## Account Allocation on the Shared Devnet

To avoid nonce collisions while sharing the remote devnet:

| Account | Address | Reserved for | Private key |
|---|---|---|---|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | P0-1 owner (keychain side) | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | **P0-2 (you)** | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |
| 2 | `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC` | Session-key role in tests | `0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a` |
| 3 | `0x90F79bf6EB2c4f870365E785982E1f101E93b906` | Bystander / recipient in transfer tests | (not commonly used) |

Mnemonic for the full set: `test test test test test test test test test test test junk`.

## When You're Ready for Path B — Producing the Genesis Alloc Fragment

A complete reproducible request looks like:

```json
{
  "0xbbbbbbbb00000000000000000000000000000001": {
    "nonce": "0x0",
    "balance": "0x0",
    "code": "0x...<full deployedBytecode>",
    "storage": {
      "0x0000000000000000000000000000000000000000000000000000000000000000": "0x..."
    }
  },
  "0xbbbbbbbb00000000000000000000000000000002": {
    "...": "..."
  },
  "0xbbbbbbbb00000000000000000000000000000003": { ... },
  "0xcccccccc00000000000000000000000000000001": { ... }
}
```

Verification steps before submission:

1. `forge build` succeeds and tests pass against a local anvil with the
   contracts deployed at runtime (Path A path).
2. `forge inspect <Contract> deployedBytecode` is reproducible.
3. `forge inspect <Contract> storageLayout` listed and you've translated
   any constructor-set fields into explicit `storage` entries.
4. JSON validates (`jq . your-fragment.json`).

Hand the fragment to the P0-1 owner with a one-line summary of what's
been added or changed since the last submission so they know whether the
devnet must restart.

## Reference

- Branch summary: [`sci/docs/feat-p0-1-keychain-branch-summary.md`](feat-p0-1-keychain-branch-summary.md)
- Architecture and conventions: [`/CLAUDE.md`](../../CLAUDE.md)
- Keychain ABI (Rust): `sci/crates/precompile-abi/src/precompiles/account_keychain.rs`
- SciAgentState ABI (Rust): `sci/crates/precompile-abi/src/precompiles/sci_agent_state.rs`
- SCIAgentDelegator ABI (Rust): `sci/crates/precompile-abi/src/predeploys/sci_agent_delegator.rs`
- Hook design notes: `CLAUDE.md` → "Pre-execution Hook Design (P0-1.7 / P0-1.8)"

## Questions or Coordination

If something here is unclear or your contract design needs a change on the
keychain side (e.g., a new precompile method, an ABI tweak), open an issue
or ping in the project channel. The contracts side (`sci/contracts/`)
and the precompile side (`sci/crates/precompiles/`) are designed to
co-evolve through PRs that touch both — that's exactly why both live in
the same repo under `sci/`.
