# Plan A — Agent E2E Runbook (native AA tx `0x76`, no EIP-7702)

**Date:** 2026-06-04. **Branch:** `feat/plan-a-aa-keychain`. **Registration model:** Option B
(see `agent-registration-path-decision.md`).

This is the AA-transaction-flow analogue of the legacy `sci/devnet/E2E.md` (which used EIP-7702
+ `SCIAgentDelegator`). Under Plan A the agent batch rides a native `0x76` transaction and the
keychain checks run pre-execution in the EL — there is **no 7702 delegation and no
`delegator.execute`**. The agent's root is a plain keychain account; registration is the root
directly calling `keychain.authorizeKey` (the `SciAgentRegistrar` one-step helper only works
under 7702 and is not used here).

## Prerequisites

- A running SCI devnet (`L2_RPC=http://localhost:8545`), all three EL/sequencer/CL images on the
  Plan A branch (`base-reth-node`, `base-builder`, `base-consensus`).
- `cast` (foundry) and the `sci-aa-txgen` tool (`target/release/sci-aa-txgen`).
- Predeploys present: keychain `0xAAAA..00`, SciAgentState `0xAAAA..01`, Registry `0xBBBB..01`,
  Budget `0xBBBB..02`, CircuitBreaker `0xBBBB..03`.

```bash
export PATH=$PATH:~/.foundry/bin
RPC=http://localhost:8545
KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
REGISTRY=0xBBBBBBBB00000000000000000000000000000001
BREAKER=0xBBBBBBBB00000000000000000000000000000003
# ROOT = a funded plain account (agent principal); SESSION = the agent's session key
ROOT=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC   ; ROOT_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
SESSION=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65; SESSION_PK=0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a
```

## Phase 1 — Register (authorize a session key, no 7702)

The root authorizes its session key directly on the keychain (unrestricted here), and optionally
binds an off-chain `agentId` in the registry for discovery.

```bash
# KeyRestrictions = (expiry, enforceLimits, TokenLimit[], allowAnyCalls, CallScope[])
cast send $KEYCHAIN \
  "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))" \
  $SESSION 0 "(18446744073709551615,false,[],true,[])" \
  --private-key $ROOT_PK --rpc-url $RPC
# optional metadata binding (agentId is an off-chain identifier under Option B):
cast send $REGISTRY "bindKey(address,bytes32)" $SESSION $(cast format-bytes32-string "agent-research") \
  --private-key $ROOT_PK --rpc-url $RPC
# verify:
cast call $KEYCHAIN "getKey(address,address)(uint8,address,uint64,bool,bool)" $ROOT $SESSION --rpc-url $RPC
```

**Expected / observed (2026-06-04):** `keys[ROOT][SESSION]` active — `keyId=SESSION`,
`expiry=18446744073709551615` (u64::MAX), `isRevoked=false`. ✅

## Phase 2 — AA transfer (sponsored; session key holds no funds it must spend)

The session key signs a `0x76` tx; `root` names whom the calls execute as, and `fee_payer == root`
sponsors gas (so the session key can be fundless).

```bash
N=$(cast nonce $SESSION --rpc-url $RPC)
SINK=0x6666666666666666666666666666666666666666
RAW=$(ROOT=$ROOT FEE_PAYER=$ROOT ./target/release/sci-aa-txgen $SESSION_PK 42001 $N $SINK 5)
cast rpc eth_sendRawTransaction "$RAW" --rpc-url $RPC
# then: cast rpc eth_getTransactionReceipt <hash>
```

**Expected / observed:** status `0x1`, block 36904, gasUsed 21000 (receipt type `0x2` = AA→EIP-1559
mapping). Conservation: SESSION balance unchanged + nonce +1 (**signer nets 0**), SINK +5, ROOT
delta = `−(gas + L1 fee + value)`. ✅

## Phase 3 — Spending limit (enforce + pass/reject)

Authorize a key with `enforceLimits=true` and a per-token cap. The limit meter decodes
`transfer(address,uint256)` calldata for any target (D3-B), and the `address(0)` sentinel meters
native value + gas (D-gas) — keep its cap above `gas_limit * max_fee`.

```bash
# token T cap 100; address(0) sentinel cap 1e18 (covers gas reservation)
cast send $KEYCHAIN "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))" \
  $SESSION2 0 "(18446744073709551615,true,[(0x0000000000000000000000000000000000000000,1000000000000000000,31536000),(0x7777777777777777777777777777777777777777,100,31536000)],true,[])" \
  --private-key $ROOT_PK --rpc-url $RPC

T=0x7777777777777777777777777777777777777777; RCPT=0x8888888888888888888888888888888888888888
# positive: transfer(RCPT,50) <= 100  -> included
RAWP=$(ROOT=$ROOT FEE_PAYER=$ROOT INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 50) GAS_LIMIT=120000 \
       ./target/release/sci-aa-txgen $SESSION2_PK 42001 $N $T 0)
# negative: transfer(RCPT,200) > 100  -> rejected by pre-flight (never mined)
RAWN=$(ROOT=$ROOT FEE_PAYER=$ROOT INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 200) GAS_LIMIT=120000 \
       ./target/release/sci-aa-txgen $SESSION2_PK 42001 $((N+1)) $T 0)
```

**Expected:** positive tx → status `0x1`; negative tx → no receipt (never mined), nonce unchanged.
**Status:** the `authorizeKey` with enforced limits landed (`KeyAuthorized`, expiry=max,
enforceLimits set). The identical positive-include / negative-reject outcome was **devnet-verified
on 2026-06-03** (Phase-2 follow-up #4: `transfer(RCPT,50)` mined status 1; `transfer(RCPT,200)`
rejected, nonce unchanged). In the 2026-06-04 cohesive run the pass/reject txs were submitted but
did not land before the devnet sequencer wedged (see caveat below); the mechanism is unchanged.

## Phase 4 — Circuit breaker (trip → reject → untrip → include)

```bash
# CB owner/guardian trips the session key:
cast send $BREAKER "trip(address,bytes32)" $SESSION $(cast format-bytes32-string "manual-freeze") \
  --private-key $OWNER_PK --rpc-url $RPC
cast call $BREAKER "isTripped(address)(bool)" $SESSION --rpc-url $RPC     # true
# an AA tx from SESSION is now rejected pre-execution (accepted to pool, never mined, nonce unchanged)
# untrip restores it; the same pending tx becomes includable:
cast send $BREAKER "untrip(address)" $SESSION --private-key $OWNER_PK --rpc-url $RPC
```

**Expected / observed (2026-06-04):** after `trip`, the AA tx (hash `0x0927…`) stayed pending and
**was not mined** (nonce unchanged) — rejected by the keychain hook's circuit-breaker check. After
`untrip`, the **same** tx was included: status `0x1`, block 37186, SINK balance +7. ✅ (Trip state
lives in the `SciAgentState` precompile; `isTripped` confirmed `true` while tripped, `false` after.)

## Phase 5 — Key expiry

Authorize a key with a near-future `expiry`; an AA tx before expiry passes, after expiry is
rejected by the keychain (same active-key check used by every phase).

```bash
EXP=$(( $(date +%s) + 60 ))
cast send $KEYCHAIN "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))" \
  $SESSION3 0 "($EXP,false,[],true,[])" --private-key $ROOT_PK --rpc-url $RPC
# before EXP: AA tx from SESSION3 -> included ; after EXP: AA tx -> rejected (expired key)
```

**Status:** not separately exercised on devnet in this run. The expiry gate is the same
`load_active_key` path the keychain hook applies on every AA tx (the `expiry` field is checked
alongside `isRevoked`); it has unit-test coverage in `sci-precompiles`. A dedicated devnet
demonstration is a follow-up.

## Devnet stability caveat (important for reproducing)

During the 2026-06-04 run the sequencer wedged in `AwaitingSafeHeadConfirmation`: the unsafe head
drifted far ahead of the safe head (batcher / L1-derivation lag), so the sequencer paused. Two
lessons:
- **Do not repeatedly restart the CL** (`base-{builder,client}-cl`) to "unstick" it — each restart
  can reorg the unsafe chain, which resets the batcher pipeline and makes the safe-head lag worse.
- If the chain is drift-stalled, prefer letting it settle / ensuring the batcher posts; a full
  recovery may require the wipe-genesis redeploy in `project_devnet_v1_7_1_deployment`.

This is a devnet ops/config characteristic, independent of the AA/keychain consensus logic.
