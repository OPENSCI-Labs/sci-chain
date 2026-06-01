---
title: "P0-2 Base Contracts — Integration Test Plan"
date: "2026-05-29"
branch: "feat/p0-2-contracts-v1.7.1"
covers:
  - "5 Solidity contracts (4 fixed-address predeploys + 1 helper)"
  - "Pre-execution hook end-to-end (CircuitBreaker / Scope / SpendingLimit)"
  - "EIP-7702 agent-tx loop wiring"
runs_against: "Live SCI devnet (chain 42001) with v1.7.1 keychain image + extended sci-allocs"
first_verified: "2026-05-29 (ubuntu@54.255.70.252, genesis 0xa29c1c…)"
---

# P0-2 Base Contracts — Integration Test Plan

This document is the executable test plan for everything that landed on
`feat/p0-2-contracts-v1.7.1`. Two complementary surfaces:

1. **Foundry integration tests** under `sci/contracts/test/integration/` —
   forge-test based, fork the live SCI devnet via `--fork-url`, exercise the
   real on-chain bytecode at the predeploy addresses, and include fuzz +
   invariant + stress coverage. Run with
   `FOUNDRY_PROFILE=integration forge test --fork-url $L2_RPC`. See
   `sci/contracts/test/integration/README.md` for the suite map.
2. **Live-broadcast cast walkthroughs** (this document, plus
   `sci/devnet/E2E.md`) — copy-paste cast snippets that exercise the SCI Rust
   pre-execution hook end-to-end. Required for §7 since the hook lives in the
   EL binary and cannot be simulated under anvil.

Every cast step below has an expected outcome, a status (verified date or
pending), and a one-line rationale.

The plan is scoped to the work shipped in P0-2:

- 5 Solidity contracts under `sci/contracts/src/` (4 fixed-address predeploys
  via genesis alloc + `SciAgentRegistrar` deployed normally)
- The pre-execution hook's CircuitBreaker / Scope / SpendingLimit checks
  routed through the new predeploys
- The EIP-7702 → `SCIAgentDelegator` agent-tx entrypoint

Tests of pre-existing P0-1 keychain ABI surface (T1-T5 + witness API) are
referenced where the agent-tx loop depends on them, but are not re-listed
here; for those see `feat-p0-1-keychain-branch-summary.md`.

---

## Conventions

```bash
# Devnet RPCs (builder is freshest; client lags via P2P).
export L2_RPC=http://localhost:7545        # builder (sequencer)
export L2_CLIENT_RPC=http://localhost:8545 # client (verifier)

# Test accounts. On a freshly-generated devnet chain, ACC0 (alice) starts at
# balance 0; ACC1 (bob) and ACC2 are funded with 10000 ETH each. Tests that
# need alice as a sender must first fund her from bob.
export ALICE=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
export ALICE_PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
export BOB=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
export BOB_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
export CHARLIE=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
export CHARLIE_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a

# SCI addresses (genesis-baked, deterministic across deploys).
export KEYCHAIN=0xaaaaaaaa00000000000000000000000000000000
export SCI_AGENT_STATE=0xaaaaaaaa00000000000000000000000000000001
export REGISTRY=0xbbbbbbbb00000000000000000000000000000001
export BUDGET=0xbbbbbbbb00000000000000000000000000000002
export BREAKER=0xbbbbbbbb00000000000000000000000000000003
export DELEGATOR=0xcccccccc00000000000000000000000000000001

# Default token symbol for spending-limit tests when no real ERC-20 is deployed.
export NATIVE=0x0000000000000000000000000000000000000000

# bytes32 helper (cast handles padding correctly; hand-padded literals are
# error-prone).
make_agent_id() { cast format-bytes32-string "$1"; }
```

Status legend:

- ✅ **verified** — passed end-to-end on the deployment of `<date>`
- ⏳ **pending** — written but not yet run against live devnet
- 🚧 **blocked** — depends on infrastructure not yet in place

---

## §1. Preflight smoke tests

These confirm the chain is in the expected post-redeploy state. Run before
any other test.

### T1.1 Chain identity — ✅ verified 2026-05-29

```bash
cast chain-id     --rpc-url $L2_RPC     # expect: 42001
cast block-number --rpc-url $L2_RPC     # expect: > 0 and increasing
```

### T1.2 Precompile markers — ✅ verified 2026-05-29

Confirms the EIP-161 GC fix (`code: "0xef"` alloc) is in place; without this,
keychain sstore writes get silently GC'd.

```bash
for addr in $KEYCHAIN $SCI_AGENT_STATE; do
  code=$(cast code $addr --rpc-url $L2_RPC)
  [ "$code" = "0xef" ] && echo "ok $addr" || echo "FAIL $addr ($code)"
done
```

### T1.3 Predeploy bytecode present — ✅ verified 2026-05-29

Each fixed-address contract should have runtime bytecode starting with the
standard Solidity dispatcher prefix `0x6080604052...`. Byte counts are
informational (current builds: registry 3314, budget 2228, breaker 3058,
delegator 2112 chars).

```bash
for addr in $REGISTRY $BUDGET $BREAKER $DELEGATOR; do
  code=$(cast code $addr --rpc-url $L2_RPC)
  if [ "${code:0:10}" = "0x60806040" ]; then
    echo "ok $addr (${#code} chars)"
  else
    echo "FAIL $addr (head=${code:0:24})"
  fi
done
```

### T1.4 AgentCircuitBreaker owner — ✅ verified 2026-05-29

The genesis alloc seeds `_owner` at storage slot 0. Default is alice
(test-account-0); override via `CB_OWNER` env to `export-predeploy-allocs.sh`
at chain init.

```bash
cast call $BREAKER "owner()(address)" --rpc-url $L2_RPC
# Expect: 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 (alice)

cast call $BREAKER "isGuardian(address)(bool)" $ALICE --rpc-url $L2_RPC
# Expect: true
```

### T1.5 Test accounts — ✅ verified 2026-05-29

```bash
cast balance $ALICE   --rpc-url $L2_RPC   # expect: 0 (fund from bob if needed)
cast balance $BOB     --rpc-url $L2_RPC   # expect: 10000000000000000000000
cast balance $CHARLIE --rpc-url $L2_RPC   # expect: 10000000000000000000000

# If alice needs gas:
cast send $ALICE --value 1ether --rpc-url $L2_RPC --private-key $BOB_PK
```

---

## §2. AccountKeychain dependencies for P0-2

These keychain methods are called by the new contracts and the hook. Re-verify
they work as P0-2 expects, then move on.

### T2.1 authorizeKey (T3 overload, unrestricted) — ✅ verified 2026-05-29

```bash
EXPIRY=$(( $(date +%s) + 86400 ))
cast send $KEYCHAIN \
  "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))" \
  $BOB 0 \
  "($EXPIRY,false,[],true,[])" \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast call $KEYCHAIN "getKey(address,address)(uint8,address,uint64,bool,bool)" \
  $ALICE $BOB --rpc-url $L2_RPC
# Expect: 0, 0x7099..., <expiry>, false, false  ← keyId=bob, isRevoked=false
```

### T2.2 revokeKey — ⏳ pending

```bash
cast send $KEYCHAIN "revokeKey(address)" $BOB \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast call $KEYCHAIN "getKey(address,address)(uint8,address,uint64,bool,bool)" \
  $ALICE $BOB --rpc-url $L2_RPC
# Expect: <sigtype>, <bob>, <expiry>, <enforce>, true  ← isRevoked=true
```

### T2.3 updateSpendingLimit — ⏳ pending

```bash
cast send $KEYCHAIN "updateSpendingLimit(address,address,uint256)" \
  $BOB $NATIVE 1000000 \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast call $KEYCHAIN "getRemainingLimit(address,address,address)(uint256)" \
  $ALICE $BOB $NATIVE --rpc-url $L2_RPC
# Expect: 1000000
```

### T2.4 T5 witness API (authorizeKey 3-arg + burnKeyAuthorizationWitness) — ⏳ pending

```bash
WITNESS=$(cast format-bytes32-string "witness-1")
cast send $KEYCHAIN \
  "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]),bytes32)" \
  $CHARLIE 0 "($EXPIRY,false,[],true,[])" $WITNESS \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: KeyAuthorizationWitness event

cast call $KEYCHAIN "isKeyAuthorizationWitnessBurned(address,bytes32)(bool)" \
  $ALICE $WITNESS --rpc-url $L2_RPC
# Expect: false (not burned)

cast send $KEYCHAIN "burnKeyAuthorizationWitness(bytes32)" $WITNESS \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: KeyAuthorizationWitnessBurned event

cast call $KEYCHAIN "isKeyAuthorizationWitnessBurned(address,bytes32)(bool)" \
  $ALICE $WITNESS --rpc-url $L2_RPC
# Expect: true
```

---

## §3. AgentAccessKeyRegistry (0xBBBB..01)

### T3.1 bindKey happy path — ✅ verified 2026-05-29

Requires T2.1 (a key must be authorized on the keychain first). Use
`cast format-bytes32-string` — hand-padded hex literals frequently lose a
trailing zero and revert silently.

```bash
AGENT_ID=$(cast format-bytes32-string "agent-1")

cast send $REGISTRY "bindKey(address,bytes32)" $BOB $AGENT_ID \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast call $REGISTRY "agentIdOf(address)(bytes32)" $BOB --rpc-url $L2_RPC
# Expect: $AGENT_ID

cast call $REGISTRY "isBound(address)(bool)" $BOB --rpc-url $L2_RPC
# Expect: true

cast call $REGISTRY "getBinding(address)((bytes32,address,uint64,bool))" $BOB \
  --rpc-url $L2_RPC
# Expect: (agentId, account=alice, registeredAt=<unix>, revoked=false)
```

### T3.2 bindKey reverts on zero inputs — ⏳ pending

```bash
# Zero keyId
cast send $REGISTRY "bindKey(address,bytes32)" \
  0x0000000000000000000000000000000000000000 $AGENT_ID \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: revert with ZeroKeyId()  selector 0x...

# Zero agentId
cast send $REGISTRY "bindKey(address,bytes32)" $BOB \
  0x0000000000000000000000000000000000000000000000000000000000000000 \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: revert with ZeroAgentId()
```

### T3.3 bindKey reverts when caller is not the keychain owner — ⏳ pending

```bash
# A stranger (charlie) tries to bind bob's key — keychain returns empty
# KeyInfo for (charlie, bob), so registry reverts with NotBound().
cast send $REGISTRY "bindKey(address,bytes32)" $BOB $AGENT_ID \
  --rpc-url $L2_RPC --private-key $CHARLIE_PK
# Expect: revert with NotBound()
```

### T3.4 Rebind reverts when already bound; unbind then bind allowed — ⏳ pending

```bash
# Already bound (from T3.1)
AGENT_ID_2=$(cast format-bytes32-string "agent-2")
cast send $REGISTRY "bindKey(address,bytes32)" $BOB $AGENT_ID_2 \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: revert with AlreadyBound()

# Unbind
cast send $REGISTRY "unbindKey(address)" $BOB \
  --rpc-url $L2_RPC --private-key $ALICE_PK
cast call $REGISTRY "isBound(address)(bool)" $BOB --rpc-url $L2_RPC
# Expect: false

# Now rebind with a different agent
cast send $REGISTRY "bindKey(address,bytes32)" $BOB $AGENT_ID_2 \
  --rpc-url $L2_RPC --private-key $ALICE_PK
cast call $REGISTRY "agentIdOf(address)(bytes32)" $BOB --rpc-url $L2_RPC
# Expect: $AGENT_ID_2
```

---

## §4. AgentBudgetController (0xBBBB..02)

### T4.1 remaining proxies to keychain — ⏳ pending

```bash
# Pre-condition: T2.3 set a remaining limit of 1_000_000 for (alice, bob, NATIVE)
cast call $BUDGET "remaining(address,address,address)(uint256,uint64)" \
  $ALICE $BOB $NATIVE --rpc-url $L2_RPC
# Expect: 1000000, 0   (matches keychain.getRemainingLimitWithPeriod)
```

### T4.2 setThreshold + getThreshold — ✅ verified 2026-05-29 (via inner call)

Confirms `_thresholds[msg.sender][keyId][token]` is keyed by the caller. The
inner-call form (via `SCIAgentDelegator.execute`) was verified end-to-end; a
direct cast `setThreshold` call is also acceptable and equivalent.

```bash
cast send $BUDGET "setThreshold(address,address,uint256)" $BOB $NATIVE 12345 \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast call $BUDGET "getThreshold(address,address,address)(uint256)" \
  $ALICE $BOB $NATIVE --rpc-url $L2_RPC
# Expect: 12345
```

### T4.3 checkAndAlert emits BudgetAlert only when remaining ≤ threshold — ⏳ pending

```bash
# With remaining=1_000_000 (from T2.3) and threshold=12345 (from T4.2),
# remaining > threshold → no alert.
cast send $BUDGET "checkAndAlert(address,address,address)(uint256,uint64,bool)" \
  $ALICE $BOB $NATIVE \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: tx succeeds, no BudgetAlert log, returns (1000000, 0, false)

# Now set threshold ABOVE remaining → alert fires.
cast send $BUDGET "setThreshold(address,address,uint256)" $BOB $NATIVE 2000000 \
  --rpc-url $L2_RPC --private-key $ALICE_PK
cast send $BUDGET "checkAndAlert(address,address,address)(uint256,uint64,bool)" \
  $ALICE $BOB $NATIVE \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: BudgetAlert log with (account=alice, keyId=bob, token=NATIVE,
#         remaining=1000000, threshold=2000000); returns (.., .., true)
```

### T4.4 setThreshold is per-account isolated — ⏳ pending

```bash
# Charlie sets a different threshold; alice's threshold unchanged.
cast send $BUDGET "setThreshold(address,address,uint256)" $BOB $NATIVE 9999 \
  --rpc-url $L2_RPC --private-key $CHARLIE_PK

cast call $BUDGET "getThreshold(address,address,address)(uint256)" \
  $ALICE   $BOB $NATIVE --rpc-url $L2_RPC   # alice's: still 12345 (or 2000000 from T4.3)
cast call $BUDGET "getThreshold(address,address,address)(uint256)" \
  $CHARLIE $BOB $NATIVE --rpc-url $L2_RPC   # charlie's: 9999
```

---

## §5. AgentCircuitBreaker (0xBBBB..03)

### T5.1 Owner trip/untrip + events — ✅ verified 2026-05-29

```bash
REASON=$(cast format-bytes32-string "manual-trip")
cast send $BREAKER "trip(address,bytes32)" $BOB $REASON \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: Tripped(bob, alice, reason) AND
#         TripStateUpdate(bob, true) from SciAgentState precompile

cast call $BREAKER         "isTripped(address)(bool)" $BOB --rpc-url $L2_RPC  # expect: true
cast call $SCI_AGENT_STATE "isTripped(address)(bool)" $BOB --rpc-url $L2_RPC  # expect: true

cast send $BREAKER "untrip(address)" $BOB \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: Untripped(bob, alice) + TripStateUpdate(bob, false)

cast call $BREAKER "isTripped(address)(bool)" $BOB --rpc-url $L2_RPC  # expect: false
```

### T5.2 Guardian model — ⏳ pending

```bash
GUARDIAN=$CHARLIE

cast send $BREAKER "setGuardian(address,bool)" $GUARDIAN true \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: GuardianUpdated(charlie, true)

cast call $BREAKER "isGuardian(address)(bool)" $GUARDIAN --rpc-url $L2_RPC
# Expect: true

# Guardian can trip
cast send $BREAKER "trip(address,bytes32)" $BOB \
  0x0000000000000000000000000000000000000000000000000000000000000000 \
  --rpc-url $L2_RPC --private-key $CHARLIE_PK
# Expect: status=1, Tripped event from CHARLIE
```

### T5.3 Unauthorized caller revert — ⏳ pending

```bash
STRANGER_PK=0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6  # ACC3

cast send $BREAKER "trip(address,bytes32)" $BOB \
  0x0000000000000000000000000000000000000000000000000000000000000000 \
  --rpc-url $L2_RPC --private-key $STRANGER_PK
# Expect: revert with UnauthorizedGuardian()
```

### T5.4 Precompile access control mirrors facade — ✅ verified 2026-05-26 (per project_devnet_v1_7_1_deployment memory)

Direct calls to `SciAgentState.tripKey/untripKey` from any address other than
the breaker contract must revert with `Unauthorized()` (selector `0x82b42900`).

```bash
cast send $SCI_AGENT_STATE "tripKey(address)" $BOB \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: revert with Unauthorized() — alice is NOT the breaker contract
```

### T5.5 setGuardian rejects zero address — ⏳ pending

```bash
cast send $BREAKER "setGuardian(address,bool)" \
  0x0000000000000000000000000000000000000000 true \
  --rpc-url $L2_RPC --private-key $ALICE_PK
# Expect: revert with ZeroAddress()
```

### T5.6 setGuardian only callable by owner — ⏳ pending

```bash
cast send $BREAKER "setGuardian(address,bool)" $CHARLIE true \
  --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: revert with OZ OwnableUnauthorizedAccount(bob)
```

---

## §6. SCIAgentDelegator (0xCCCC..01)

The delegator is exercised indirectly through the hook in §7. A direct call
test confirms its fail-closed property in isolation.

### T6.1 Direct call without hook → MissingTransactionKey revert — ⏳ pending

Calling `execute()` on the delegator address as a regular contract (no 7702
delegation, no hook firing) must revert because `getTransactionKey()` is zero.
This is the second layer of defense behind the hook.

```bash
INNER=$(cast calldata "getThreshold(address,address,address)" $ALICE $BOB $NATIVE)
OUTER=$(cast calldata "execute((address,uint256,bytes)[])" "[($BUDGET,0,$INNER)]")

cast send $DELEGATOR --data $OUTER \
  --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: revert with MissingTransactionKey() selector
```

---

## §7. Pre-execution hook (full agent-tx loop)

This is the central P0-2 integration surface. Each test exercises a different
path through `SciHandler::validate_against_state_and_deduct_caller` ▸
`sci_precompiles::run_pre_execution_hook`.

### T7.1 Happy-path agent-tx — ✅ verified 2026-05-29

Pre-conditions: T2.1 (key authorized), T3.1 (key bound), 7702 delegation in
place (see snippet in T7.X-prereq below).

**T7.X-prereq — install 7702 delegation on alice's account.** Critical: see
CLAUDE.md "EIP-7702 self-auth nonce trap (cast)" for why `--nonce` is required.

```bash
NEXT_NONCE=$(( $(cast nonce $ALICE --rpc-url $L2_RPC) + 1 ))
ALICE_AUTH=$(cast wallet sign-auth $DELEGATOR \
  --private-key $ALICE_PK --rpc-url $L2_RPC --nonce $NEXT_NONCE)
cast send $ALICE --value 0 --auth "$ALICE_AUTH" \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast code $ALICE --rpc-url $L2_RPC
# Expect: 0xef0100cccccccc00000000000000000000000000000001
```

Then send the agent batch. Inner call is `BudgetController.setThreshold` so
the side-effect is observable.

```bash
INNER=$(cast calldata "setThreshold(address,address,uint256)" $BOB $NATIVE 12345)
OUTER=$(cast calldata "execute((address,uint256,bytes)[])" "[($BUDGET,0,$INNER)]")

cast send $ALICE --data $OUTER --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: status=1, logs include:
#   ThresholdConfigured(alice, bob, NATIVE, 12345)  ← from BudgetController
#   AgentCallExecuted(alice, 0, BUDGET, 0)          ← from delegator
#   AgentBatchExecuted(alice, bob, 1)               ← from delegator

cast call $BUDGET "getThreshold(address,address,address)(uint256)" \
  $ALICE $BOB $NATIVE --rpc-url $L2_RPC
# Expect: 12345
```

### T7.2 Hook rejects tx on tripped session key — ✅ verified 2026-05-29

```bash
cast send $BREAKER "trip(address,bytes32)" $BOB \
  $(cast format-bytes32-string "test") \
  --rpc-url $L2_RPC --private-key $ALICE_PK

# Retry the same execute as in T7.1, with a different threshold (99999) to
# distinguish "would-have-applied" from "actually-applied".
INNER=$(cast calldata "setThreshold(address,address,uint256)" $BOB $NATIVE 99999)
OUTER=$(cast calldata "execute((address,uint256,bytes)[])" "[($BUDGET,0,$INNER)]")

cast send $ALICE --data $OUTER --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: estimateGas error with body
#   "SCI hook rejected tx: Fatal(\"agent session key 0x7099... is tripped\")"
# Tx NEVER lands on chain. Threshold remains at T7.1's value.

cast call $BUDGET "getThreshold(address,address,address)(uint256)" \
  $ALICE $BOB $NATIVE --rpc-url $L2_RPC
# Expect: 12345 (unchanged)
```

### T7.3 Untrip restores normal flow — ✅ verified 2026-05-29

```bash
cast send $BREAKER "untrip(address)" $BOB \
  --rpc-url $L2_RPC --private-key $ALICE_PK

# Same calldata as T7.2 — should now succeed.
cast send $ALICE --data $OUTER --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: status=1, threshold updates to 99999.

cast call $BUDGET "getThreshold(address,address,address)(uint256)" \
  $ALICE $BOB $NATIVE --rpc-url $L2_RPC
# Expect: 99999
```

### T7.4 Hook rejects tx on revoked key — ⏳ pending

```bash
cast send $KEYCHAIN "revokeKey(address)" $BOB \
  --rpc-url $L2_RPC --private-key $ALICE_PK

cast send $ALICE --data $OUTER --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: "SCI hook rejected tx" with a message indicating the key is no
#         longer active (key_is_active returns false → hook treats as
#         not-an-agent-tx; the call would then fall through and the EOA
#         self-call to a 7702-delegated address with execute() calldata
#         hits MissingTransactionKey in the delegator).
```

Two valid outcomes here, both acceptable:

- a) Hook returns `Pass` (key inactive ⇒ not an agent tx), the call goes to
     the delegator at alice's address, delegator reads `getTransactionKey()=0`,
     reverts with `MissingTransactionKey()`. The tx still lands on chain but
     status=0.
- b) Hook treats inactive key as a rejection. Tx fails before EVM execution.

Either is correct fail-closed behavior; record which one this stack actually
produces.

### T7.5 Hook rejects tx on expired key — ⏳ pending

Same shape as T7.4 but uses time-passing rather than revocation. Set the key
expiry to `block.timestamp + 60`, sleep 90 seconds, attempt execute.

### T7.6 Hook rejects tx on scope violation — ⏳ pending

Configure the key with `allowAnyCalls=false` and a single `CallScope` for a
specific target (e.g. `BUDGET` only). Then attempt an `execute(Call[])`
batch whose inner call targets a different contract.

```bash
# Re-authorize bob with scope restricted to BUDGET only, transfer-like selectors
SCOPED_RESTRICTIONS="($(( $(date +%s) + 86400 )),false,[],false,[($BUDGET,[])])"
# Note: empty selectorRules array means ANY selector on $BUDGET is allowed.
# To restrict to one selector, encode [(0xb715dcc6,[])] for setThreshold.

cast send $KEYCHAIN \
  "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))" \
  $BOB 0 "$SCOPED_RESTRICTIONS" \
  --rpc-url $L2_RPC --private-key $ALICE_PK

# Now attempt a call to REGISTRY (not in scope) — expect hook reject.
INNER_OUT_OF_SCOPE=$(cast calldata "agentIdOf(address)" $BOB)
OUTER_OOS=$(cast calldata "execute((address,uint256,bytes)[])" "[($REGISTRY,0,$INNER_OUT_OF_SCOPE)]")
cast send $ALICE --data $OUTER_OOS --rpc-url $L2_RPC --private-key $BOB_PK
# Expect: hook rejection on scope check
```

### T7.7 Hook rejects tx on spending-limit exceeded — ⏳ pending

Set a small spending limit on the keychain, then submit a batch whose
transfer/approve calldata sums beyond the limit. Use a deployed ERC-20 or
`ISCI20.transferWithMemo` for the inner call.

```bash
# 1. Deploy a mock ERC-20 (or skip if a fee token is already at known address).
# 2. authorizeKey with TokenLimit{ token=ERC20, amount=100, period=0 }.
# 3. Build execute() with a single transfer of 1000.
# 4. cast send → expect hook rejection on pre-flight pessimistic-deduction sum.
```

This test is blocked on having a deployable ERC-20 fixture; defer until that
exists in `sci/contracts/test/mocks/`.

### T7.8 Hook bypass for non-agent tx — ✅ verified 2026-05-29 (the fund-alice tx)

A regular bob→alice transfer (no 7702 delegation in effect, no execute()
calldata) must pass straight through with 21k gas. The fund-alice step in
§1 setup exercises this; tx 0x1b1890b5... used 21000 gas — the fast path.

### T7.9 Hook bypass for deposit-tx — ⏳ pending

OP-Stack system deposit transactions (type 0x7E) must short-circuit out of
the hook. These run every block to tick the L1Block predeploy and similar;
if the hook intercepted them, the chain would halt. Verify by inspecting
that L1Block's storage slot 0 advances each L1 block as observed by
`cast storage 0x4200000000000000000000000000000000000015 0x00`.

---

## §8. End-to-end gold path

This is the canonical sequence to demo to a stakeholder. Combines §3, §5, §7.
Run it from a known-clean state (fresh devnet redeploy is ideal).

### T8.1 register → execute → trip → execute (rejected) → untrip → execute — ✅ verified 2026-05-29

Reference recording from the 2026-05-29 deployment (tx hashes for audit):

| Phase | Tx hash | Gas | Status |
|---|---|---|---|
| Fund alice | `0x1b1890b5a314…3c39e75e` | 21000 | success |
| authorizeKey | `0x84c9a24532…b15242` | 51658 | success |
| bindKey | `0x4d22d817ab…29686` | 74620 | success |
| 7702 delegate | `0x052ff82724…736a71d0` | 36844 | success |
| execute #1 | `0x8913cfe8ad…b59ce7` | 56513 | success |
| trip | `0xd597ef851c…fcc3c8` | 52405 | success |
| execute #2 | (rejected by hook) | — | estimateGas error |
| untrip | (success) | 34784 | success |
| execute #3 | (success) | 39425 | success |

Genesis hash:
`0xa29c1c033ec31576fd025ed1065b46abb46cf7acba7745c864152b3250de846b`

---

## §9. Regression matrix

| ID | Surface | Status | Comment |
|---|---|---|---|
| T1.1 | Chain identity | ✅ 2026-05-29 | |
| T1.2 | Precompile markers (0xef) | ✅ 2026-05-29 | |
| T1.3 | Predeploy bytecode present | ✅ 2026-05-29 | |
| T1.4 | CB owner correct | ✅ 2026-05-29 | |
| T1.5 | Account funding state | ✅ 2026-05-29 | alice=0 by design |
| T2.1 | keychain.authorizeKey | ✅ 2026-05-29 | |
| T2.2 | keychain.revokeKey | ⏳ | |
| T2.3 | keychain.updateSpendingLimit | ⏳ | |
| T2.4 | keychain.witness API | ⏳ | T-W1..3 ✅ 2026-05-26 |
| T3.1 | registry.bindKey happy | ✅ 2026-05-29 | |
| T3.2 | registry.bindKey zero-args | ⏳ | |
| T3.3 | registry.bindKey not-owner | ⏳ | |
| T3.4 | registry.unbindKey + rebind | ⏳ | |
| T4.1 | budget.remaining proxy | ⏳ | |
| T4.2 | budget.setThreshold | ✅ 2026-05-29 | via inner call (T7.1) |
| T4.3 | budget.checkAndAlert | ⏳ | |
| T4.4 | budget per-account isolation | ⏳ | |
| T5.1 | cb.trip/untrip + events | ✅ 2026-05-29 | |
| T5.2 | cb guardian model | ⏳ | |
| T5.3 | cb unauthorized revert | ⏳ | |
| T5.4 | precompile access control | ✅ 2026-05-26 | from prior deployment |
| T5.5 | cb.setGuardian(0) rejects | ⏳ | |
| T5.6 | cb.setGuardian owner-only | ⏳ | |
| T6.1 | delegator direct call rejects | ⏳ | |
| T7.1 | Hook happy path | ✅ 2026-05-29 | |
| T7.2 | Hook rejects tripped | ✅ 2026-05-29 | |
| T7.3 | Untrip restores flow | ✅ 2026-05-29 | |
| T7.4 | Hook + revoked key | ⏳ | |
| T7.5 | Hook + expired key | ⏳ | |
| T7.6 | Hook + scope violation | ⏳ | |
| T7.7 | Hook + spending-limit exceeded | 🚧 | needs ERC-20 fixture |
| T7.8 | Hook fast-path for non-agent | ✅ 2026-05-29 | implicit in fund-alice |
| T7.9 | Hook + deposit-tx bypass | ⏳ | |
| T8.1 | End-to-end gold path | ✅ 2026-05-29 | |

Verified coverage at first ship: **13 / 33** tests run end-to-end. Remaining
**20 / 33** are documented but not yet exercised; T7.7 is blocked on an
ERC-20 deploy fixture.

---

## Appendix A — Common cast gotchas observed during P0-2 testing

### A.1 EIP-7702 self-auth nonce trap

The single most expensive bug in this round of integration testing. Symptoms,
fix, and rationale: see CLAUDE.md "Common Tasks → EIP-7702 self-auth nonce
trap (cast)".

### A.2 bytes32 encoding from short strings

Hand-padding `"agent-1"` to bytes32 by typing the hex literal is error-prone
(off-by-one zero produces a 31-byte argument that cast silently truncates and
the tx then reverts unintelligibly). Always use:

```bash
AGENT_ID=$(cast format-bytes32-string "agent-1")
```

### A.3 KeyInfo struct shape

`AccountKeychain.getKey` returns **5** fields, not 4. The current CLAUDE.md
example shows `(uint8,uint64,bool,bool)`, which is missing the `keyId` field.
Use:

```bash
cast call $KEYCHAIN "getKey(address,address)(uint8,address,uint64,bool,bool)" ...
```

### A.4 Nested tuple in KeyRestrictions

The 5-tuple `KeyRestrictions` has two array-of-tuple fields. Cast accepts
this syntax:

```bash
"($EXPIRY,$ENFORCE,[],$ALLOW_ANY,[])"
# or with non-empty arrays:
"($EXPIRY,false,[($TOKEN,$AMOUNT,$PERIOD)],false,[($TARGET,[($SELECTOR,[$RECIPIENT])])])"
```

Spaces inside the tuple are tolerated; trailing commas are not.

### A.5 RPC choice: builder vs client

The builder (`:7545`) is the freshest head; the client (`:8545`) lags via
P2P. For tests that read state immediately after a write, prefer `:7545`. For
finality-relevant checks (verifier acceptance), read both and compare.

---

## Appendix B — Resetting state between test suites

Tests in §3-§7 share state on the keychain and registry. To re-run a suite
from scratch without a full devnet redeploy:

```bash
# Revoke + re-authorize the test key (resets KeyInfo and clears scope/limits).
cast send $KEYCHAIN "revokeKey(address)" $BOB --rpc-url $L2_RPC --private-key $ALICE_PK
cast send $KEYCHAIN "authorizeKey(...)" ... --rpc-url $L2_RPC --private-key $ALICE_PK

# Unbind + rebind in the registry.
cast send $REGISTRY "unbindKey(address)" $BOB --rpc-url $L2_RPC --private-key $ALICE_PK

# Untrip if still tripped.
cast send $BREAKER "untrip(address)" $BOB --rpc-url $L2_RPC --private-key $ALICE_PK

# Revoke 7702 delegation by signing an auth for address(0) and sending it.
NEXT_NONCE=$(( $(cast nonce $ALICE --rpc-url $L2_RPC) + 1 ))
REVOKE_AUTH=$(cast wallet sign-auth 0x0000000000000000000000000000000000000000 \
  --private-key $ALICE_PK --rpc-url $L2_RPC --nonce $NEXT_NONCE)
cast send $ALICE --value 0 --auth "$REVOKE_AUTH" \
  --rpc-url $L2_RPC --private-key $ALICE_PK
cast code $ALICE --rpc-url $L2_RPC   # expect: 0x
```

For a complete reset (genesis included), run the redeploy workflow from
`project-devnet-v1-7-1-deployment` memory + the new
`sci/devnet/apply-predeploy-allocs.sh` step.
