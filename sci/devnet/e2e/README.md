# Plan A — Agent AA-tx end-to-end scripts (devnet)

Executable end-to-end scripts for the Plan A agent loop (native AA tx `0x76`, **no
EIP-7702**). These are the runnable counterpart of the spec in
`sci/docs/plan-a-aa-e2e.md` (which documents each phase + expected output). They replace
the removed Plan B (EIP-7702 + delegator) flow.

| Script | What it checks |
|---|---|
| `e2e-loop.sh` | Full agent loop **P1–P5**: register (authorizeKey) → AA transfer (sponsored, signer nets 0) → spending-limit pass/reject → circuit-breaker trip/untrip → key expiry before/after. |
| `reject-test.sh` | Regression guard: a single keychain-hook-**rejected** AA tx must NOT wedge the chain (builder skips it as `InvalidTransaction`; head keeps advancing). Guards commit `25c485a92`. |

## Prerequisites

- A running SCI devnet on the Plan A branch (`feat/plan-a-aa-keychain`), all three EL /
  sequencer / CL images built from it, with the SCI predeploys present
  (`0xAAAA..00/01`, `0xBBBB..01/02/03`).
- `cast` (foundry) + the `sci-aa-txgen` tool built: `cargo build --release -p sci-aa-txgen`
  (produces `target/release/sci-aa-txgen`).
- Test accounts funded per the standard mnemonic (`test test ... junk`).

## Running

```bash
# defaults: L2_RPC=http://localhost:8545, repo inferred, txgen at target/release/sci-aa-txgen
sci/devnet/e2e/e2e-loop.sh

# against a remote/forwarded devnet, or the sequencer directly (bypasses the verifier
# sendRawTransaction proxy — recommended for a clean single-node run):
L2_RPC=http://localhost:7545 L2_NODE_RPC=http://localhost:7549 sci/devnet/e2e/reject-test.sh
```

Env overrides (both scripts): `L2_RPC`, `SCI_REPO`, `AA_TXGEN`, `CHAIN_ID`
(`reject-test.sh` also takes `L2_NODE_RPC` for the op-node `optimism_syncStatus` poll).

## Status

P1–P5 were verified green end-to-end on devnet 2026-06-04 (commit `25c485a92`), chain
never wedged; `reject-test.sh` confirmed the same unauthorized-root AA tx that froze the
head pre-fix now leaves the chain producing. See `sci/docs/plan-a-aa-e2e.md` for the
recorded per-phase outputs and the devnet-stability caveat.
