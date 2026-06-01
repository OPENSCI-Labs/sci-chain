# SCI Base Contracts — End-to-end Devnet Walkthrough

This document walks through a manual `register → trip → execute` flow against a
running SCI devnet, using `cast` from the host shell. It assumes the devnet has the
4 SCI Solidity predeploys baked into genesis at their fixed addresses, alongside
the 2 SCI precompile-marker addresses.

The walkthrough is deliberately copy-paste oriented; each command is followed by
the expected response shape so you can sanity-check as you go.

---

## 0. Pre-flight: rebuild devnet with the extended allocs

From the devnet host (the machine that runs the docker compose stack):

```bash
# 1. Generate L2 genesis as usual (e.g. `just devnet up-single` will do this; or
#    run setup-l2 directly so we can intercept genesis.json before the node boots).
just devnet setup-l2

# 2. Bake the SCI precompile markers (existing).
bash ~/sci-dev/sci-chain/sci/devnet/apply-sci-allocs.sh \
  .devnet/l2/configs/genesis.json

# 3. Bake the 4 Solidity predeploys (NEW). CB_OWNER defaults to test-account-0;
#    override if you want a different AgentCircuitBreaker owner.
bash ~/sci-dev/sci-chain/sci/devnet/apply-predeploy-allocs.sh \
  .devnet/l2/configs/genesis.json

# 4. Start the stack.
just devnet up-single
```

Verify the four predeploys are live:

```bash
export L2_RPC=http://localhost:8545

for addr in \
  0xbbbbbbbb00000000000000000000000000000001 \
  0xbbbbbbbb00000000000000000000000000000002 \
  0xbbbbbbbb00000000000000000000000000000003 \
  0xcccccccc00000000000000000000000000000001; do
  echo -n "$addr  "
  cast code "$addr" --rpc-url "$L2_RPC" | head -c 12 ; echo " ..."
done
```

Each address should return runtime bytecode starting with `0x60806040`.

---

## 1. Setup: addresses, accounts, env

```bash
# Predeploy addresses (lowercase form ok for cast).
export KEYCHAIN=0xaaaaaaaa00000000000000000000000000000000
export SCI_AGENT_STATE=0xaaaaaaaa00000000000000000000000000000001
export REGISTRY=0xbbbbbbbb00000000000000000000000000000001
export BUDGET=0xbbbbbbbb00000000000000000000000000000002
export BREAKER=0xbbbbbbbb00000000000000000000000000000003
export DELEGATOR=0xcccccccc00000000000000000000000000000001

# Devnet test accounts (mnemonic: test test test ... junk).
export ALICE=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266   # root account
export ALICE_PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80

# Session key (Bob): treat as Alice's agent session key, funded for gas only.
export BOB=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
export BOB_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d

# Inner-call sink: deploy a tiny contract to receive batch calls. We use Bob's
# wallet so the deployment doesn't interfere with Alice's nonce later.
forge create --rpc-url "$L2_RPC" --private-key "$BOB_PK" \
  --broadcast \
  contracts/lib/forge-std/src/Test.sol:Test
# OR (recommended): write a 5-line Counter.sol in your scratch dir and deploy it.
export SINK=0x<address-from-the-deploy>

# Demo agentId.
export AGENT_ID=0x6167656e742d31000000000000000000000000000000000000000000000000   # "agent-1"
```

---

## 2. Register: authorize session key + bind to agentId + EIP-7702 delegate

### 2a. Alice authorizes Bob on the keychain (unrestricted, 1-day expiry)

```bash
# T3 authorizeKey overload: (keyId, signatureType, KeyRestrictions).
# Signature type 0 = Secp256k1. allowAnyCalls=true => no scope restriction.
cast send "$KEYCHAIN" \
  "authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))" \
  "$BOB" 0 \
  "($(($(date +%s) + 86400)),false,[],true,[])" \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"
```

Verify:

```bash
cast call "$KEYCHAIN" \
  "getKey(address,address)(uint8,address,uint64,bool,bool)" \
  "$ALICE" "$BOB" --rpc-url "$L2_RPC"
# Returns: (0, 0x7099..., <expiry>, false, false)  ← isRevoked=false
```

### 2b. Alice binds Bob → agentId in the registry

```bash
cast send "$REGISTRY" "bindKey(address,bytes32)" "$BOB" "$AGENT_ID" \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"

cast call "$REGISTRY" "agentIdOf(address)(bytes32)" "$BOB" --rpc-url "$L2_RPC"
# Returns: 0x6167656e742d31...  (==AGENT_ID)
```

### 2c. Alice EIP-7702-delegates her account to SCIAgentDelegator

**Important — self-auth nonce trap.** When the authorizer of the 7702 entry is the
same EOA sending the type-4 tx (our case: Alice is both), `cast wallet sign-auth`
defaults to the current account nonce, but EIP-7702 processes auth entries *after*
the tx increments the sender's nonce. The auth's `nonce` must equal
`current_nonce + 1`. If you omit `--nonce`, the tx still goes out as type-4 with
`status=1` and burns ~46k gas, but the delegation is **silently discarded** —
`cast code "$ALICE"` stays `0x`. Pass `--nonce` explicitly:

```bash
NEXT_NONCE=$(( $(cast nonce "$ALICE" --rpc-url "$L2_RPC") + 1 ))

# Alice signs a 7702 authorization for the delegator with the post-increment nonce.
ALICE_AUTH=$(cast wallet sign-auth "$DELEGATOR" \
  --private-key "$ALICE_PK" --rpc-url "$L2_RPC" \
  --nonce "$NEXT_NONCE")

# Submit a type-4 tx (self-call with auth list). Any tx with --auth will install
# the delegation header on the authorizer's account. We send a 0-value self-call
# from Alice; the call itself does nothing, but the auth list takes effect.
cast send "$ALICE" --value 0 \
  --auth "$ALICE_AUTH" \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"

# Verify Alice's account now carries the 7702 header.
cast code "$ALICE" --rpc-url "$L2_RPC"
# Expect: 0xef0100cccccccc00000000000000000000000000000001
```

Cross-authorization (a different EOA submits the tx after Alice signs) does NOT
need the +1 — Alice's nonce is not incremented by someone else's tx.

---

## 3. Execute (happy path)

```bash
# Encode the batch: a single inner call to $SINK with no value and 4 bytes of data.
DATA=$(cast calldata "execute((address,uint256,bytes)[])" \
  "[($SINK,0,0xdeadbeef)]")

# Bob (the session key) sends a tx to Alice's address with that calldata.
# Because Alice has delegated to the delegator, the delegator code runs at her
# address. The Rust pre-execution hook detects (7702 to delegator) + (active key
# for (Alice, Bob)), sets transaction_key=Bob, runs scope checks, and on pass
# allows EVM execution. execute() reads transaction_key (non-zero), iterates
# calls, and forwards to $SINK.
cast send "$ALICE" --rpc-url "$L2_RPC" --private-key "$BOB_PK" \
  --data "$DATA"

# Expected: tx succeeds. Look for AgentBatchExecuted log in the receipt.
```

---

## 4. Trip: emergency freeze

```bash
# Alice (owner of the CB) trips Bob's session key.
cast send "$BREAKER" "trip(address,bytes32)" "$BOB" 0x6d616e75616c000000000000000000000000000000000000000000000000000000 \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"

cast call "$BREAKER" "isTripped(address)(bool)" "$BOB" --rpc-url "$L2_RPC"
# Returns: true
```

Try to execute again — should fail in the hook:

```bash
cast send "$ALICE" --rpc-url "$L2_RPC" --private-key "$BOB_PK" --data "$DATA"
# Expected: tx error from the pre-execution hook ("agent session key … is tripped").
# The tx still pays intrinsic gas; no inner call ran.
```

---

## 5. Untrip + execute again

```bash
cast send "$BREAKER" "untrip(address)" "$BOB" \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"

cast call "$BREAKER" "isTripped(address)(bool)" "$BOB" --rpc-url "$L2_RPC"
# Returns: false

# Re-run the execute — should succeed again.
cast send "$ALICE" --rpc-url "$L2_RPC" --private-key "$BOB_PK" --data "$DATA"
```

---

## 6. Optional: budget alert

```bash
# Configure a threshold and check.
cast send "$BUDGET" "setThreshold(address,address,uint256)" \
  "$BOB" 0x0000000000000000000000000000000000000000 1000 \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"

# checkAndAlert is non-view; it emits BudgetAlert if remaining <= threshold.
cast send "$BUDGET" "checkAndAlert(address,address,address)" \
  "$ALICE" "$BOB" 0x0000000000000000000000000000000000000000 \
  --rpc-url "$L2_RPC" --private-key "$ALICE_PK"
```

(With Alice having no configured spending limit on the zero address, remaining = 0
and the alert will fire on any non-zero threshold. Use a real ERC-20 address +
authorizeKey with TokenLimit for a realistic flow.)

---

## Known caveats

1. **Bytecode size** of the delegator and registry is ~1 KB each. Genesis-alloc baking
   is fine, but if you ever change the source you must regenerate via
   `apply-predeploy-allocs.sh` and reboot the chain (allocs are only honored at chain
   init).
2. **EIP-7702 authorization replay**: once Alice delegates, the authorization stays in
   effect until she submits a new auth pointing to `address(0)` (revokes it). The
   walkthrough doesn't show revocation; do that with
   `cast wallet sign-auth 0x0000...0000 --private-key $ALICE_PK` followed by a
   self-tx with that auth.
3. **session_key gas funding**: Bob must hold ETH on devnet to pay for his own tx.
   The devnet typically funds test accounts 0–9 with 10000 ETH each, so Bob already
   has gas. If you use a fresh session key, fund it first via
   `cast send $SESSION_KEY --value 1ether --private-key $ALICE_PK`.
4. **The hook is silent on non-agent txs**. A regular tx from Alice (no 7702
   delegation in effect) or from Bob (target is not 7702-delegated to the delegator)
   passes through with no keychain enforcement. Only the (7702→delegator) AND
   (registered key) combination activates the hook.
