#!/usr/bin/env bash
# SCI Chain @ Sepolia — smoke test (highest-priority paths).
# Covers: (1) derivation consistency, (2) keychain liveness + real 0x76 AA tx,
#         (3) L1->L2 deposit-path liveness.
# Run ON THE DEPLOY HOST (needs localhost RPCs + cast + sci-aa-txgen).
#   bash sci/docs/test/smoke-test.sh
# Override endpoints via env if needed. Exits non-zero if any check fails.
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

L2_VAL=${L2_VAL:-http://localhost:8545}     # validator EL
L2_SEQ=${L2_SEQ:-http://localhost:7545}     # sequencer EL
OPNODE=${OPNODE:-http://localhost:7549}     # sequencer op-node
L1=${L1:-http://localhost:8645}             # Sepolia geth
TXGEN=${TXGEN:-$HOME/sci-dev/sci-chain/target/release/sci-aa-txgen}
CHAIN_ID=42001

DEV0=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
DEV0_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DEV1=0x70997970C51812dc3A010C7d01b50e0d17dc79C8

KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
SCISTATE=0xAAAAAAAA00000000000000000000000000000001
CB=0xBBBBBBBB00000000000000000000000000000003
L1BLOCK=0x4200000000000000000000000000000000000015
PORTAL=0xd4b05f9944dd530965e0a7cd66af205e13b69036   # OptimismPortal on Sepolia L1

PASS=0; FAIL=0
ok(){ echo "  PASS: $1"; PASS=$((PASS+1)); }
no(){ echo "  FAIL: $1"; FAIL=$((FAIL+1)); }
hr(){ echo; echo "== $1 =="; }

# ---------------------------------------------------------------------------
hr "1. Derivation consistency"
SS=$(cast rpc optimism_syncStatus --rpc-url "$OPNODE" 2>/dev/null)
read -r UNSAFE SAFE CUR_L1 <<<"$(echo "$SS" | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d["unsafe_l2"]["number"],d["safe_l2"]["number"],d["current_l1"]["number"])' 2>/dev/null)"
echo "  unsafe=$UNSAFE safe=$SAFE current_l1=$CUR_L1"
[ -n "${SAFE:-}" ] && [ "${SAFE:-0}" -gt 0 ] && ok "safe head > 0 ($SAFE)" || no "safe head not advancing (got '${SAFE:-}')"

# safe head monotonic over ~8s
sleep 8
SAFE2=$(cast rpc optimism_syncStatus --rpc-url "$OPNODE" 2>/dev/null | python3 -c 'import sys,json;print(json.load(sys.stdin)["safe_l2"]["number"])' 2>/dev/null)
[ -n "${SAFE2:-}" ] && [ "${SAFE2:-0}" -ge "${SAFE:-0}" ] && ok "safe head monotonic ($SAFE -> $SAFE2)" || no "safe head not progressing ($SAFE -> ${SAFE2:-?})"

# same block on sequencer EL and validator EL = identical derived chain
N=$(( SAFE > 5 ? SAFE - 5 : 1 ))
HS=$(cast block "$N" --rpc-url "$L2_SEQ" --field hash 2>/dev/null)
HV=$(cast block "$N" --rpc-url "$L2_VAL" --field hash 2>/dev/null)
RS=$(cast block "$N" --rpc-url "$L2_SEQ" --field stateRoot 2>/dev/null)
RV=$(cast block "$N" --rpc-url "$L2_VAL" --field stateRoot 2>/dev/null)
echo "  block $N seq.hash=$HS val.hash=$HV"
[ -n "$HS" ] && [ "$HS" = "$HV" ] && ok "block $N hash matches across seq/val EL" || no "block $N hash mismatch ($HS vs $HV)"
[ -n "$RS" ] && [ "$RS" = "$RV" ] && ok "block $N stateRoot matches across seq/val EL" || no "block $N stateRoot mismatch"

# ---------------------------------------------------------------------------
hr "2. Keychain + SCI predeploys"
codelen(){ local c; c=$(cast code "$1" --rpc-url "$L2_VAL" 2>/dev/null); echo "${#c}"; }
[ "$(cast code $KEYCHAIN --rpc-url $L2_VAL 2>/dev/null)" = "0xef" ] && ok "AccountKeychain precompile = 0xef" || no "keychain precompile code wrong"
[ "$(cast code $SCISTATE --rpc-url $L2_VAL 2>/dev/null)" = "0xef" ] && ok "SciAgentState precompile = 0xef" || no "sci_agent_state precompile code wrong"
for a in 0xBBBBBBBB00000000000000000000000000000001 0xBBBBBBBB00000000000000000000000000000002 0xBBBBBBBB00000000000000000000000000000003 0x4200000000000000000000000000000000000029 0x420000000000000000000000000000000000002A; do
  [ "$(codelen $a)" -gt 10 ] && ok "code present at $a" || no "no code at $a"
done
# getKey responds (decodes) for an unauthorized pair
if cast call $KEYCHAIN 'getKey(address,address)(uint8,uint64,bool,bool)' $DEV0 $DEV1 --rpc-url "$L2_VAL" >/dev/null 2>&1; then
  ok "keychain getKey() responds"; else no "keychain getKey() reverted"; fi
# CircuitBreaker not tripped for a fresh key
TRIP_S=$(cast call $SCISTATE 'isTripped(address)(bool)' $DEV1 --rpc-url "$L2_VAL" 2>/dev/null)
TRIP_C=$(cast call $CB 'isTripped(address)(bool)' $DEV1 --rpc-url "$L2_VAL" 2>/dev/null)
[ "$TRIP_S" = "false" ] && ok "SciAgentState.isTripped=false" || no "SciAgentState.isTripped='$TRIP_S'"
[ "$TRIP_C" = "false" ] && ok "AgentCircuitBreaker.isTripped=false" || no "AgentCircuitBreaker.isTripped='$TRIP_C'"

# ---------------------------------------------------------------------------
hr "3. AA tx type 0x76 (real end-to-end)"
if [ -x "$TXGEN" ]; then
  NONCE=$(cast nonce $DEV0 --rpc-url "$L2_VAL" 2>/dev/null)
  BAL_BEFORE=$(cast balance $DEV1 --rpc-url "$L2_VAL" 2>/dev/null)
  RAW=$(GAS_LIMIT=200000 "$TXGEN" "$DEV0_KEY" "$CHAIN_ID" "$NONCE" "$DEV1" 1000000000000000 2>/dev/null | grep -oE '0x[0-9a-fA-F]{120,}' | tail -1)
  if [ -n "$RAW" ]; then
    TXH=$(cast rpc eth_sendRawTransaction "$RAW" --rpc-url "$L2_VAL" 2>/dev/null | tr -d '"')
    echo "  0x76 tx hash=$TXH (nonce=$NONCE)"
    ST=""
    for _ in $(seq 1 20); do
      ST=$(cast rpc eth_getTransactionReceipt "$TXH" --rpc-url "$L2_VAL" 2>/dev/null \
           | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get("status","") if d else "")' 2>/dev/null)
      [ -n "$ST" ] && break
      sleep 2
    done
    [ "$ST" = "0x1" ] && ok "0x76 AA tx included, status=1" || no "0x76 AA tx not confirmed (status='$ST')"
    BAL_AFTER=$(cast balance $DEV1 --rpc-url "$L2_VAL" 2>/dev/null)
    python3 -c "exit(0 if int('$BAL_AFTER')-int('$BAL_BEFORE')==10**15 else 1)" 2>/dev/null \
      && ok "first-call value transfer applied (+0.001 SCI)" || no "value delta unexpected ($BAL_BEFORE -> $BAL_AFTER)"
  else
    no "sci-aa-txgen produced no raw tx"
  fi
else
  echo "  SKIP: sci-aa-txgen not found at $TXGEN"
fi

# ---------------------------------------------------------------------------
hr "4. L1->L2 deposit path liveness"
L2_L1NUM=$(cast call $L1BLOCK 'number()(uint64)' --rpc-url "$L2_VAL" 2>/dev/null | awk '{print $1}')
SEPOLIA_HEAD=$(cast block-number --rpc-url "$L1" 2>/dev/null | awk '{print $1}')
echo "  L2.L1Block.number=$L2_L1NUM  Sepolia head=$SEPOLIA_HEAD"
if [ -n "${L2_L1NUM:-}" ] && [ -n "${SEPOLIA_HEAD:-}" ] && [ "${L2_L1NUM:-0}" -gt 0 ]; then
  DIFF=$(( SEPOLIA_HEAD - L2_L1NUM ))
  [ "$DIFF" -ge 0 ] && [ "$DIFF" -le 60 ] && ok "L1 attributes deposit tracking Sepolia (lag ${DIFF} blocks)" || no "L1Block lag out of range (${DIFF})"
else
  no "could not read L1Block.number / Sepolia head"
fi
PORTAL_CODE=$(cast code $PORTAL --rpc-url "$L1" 2>/dev/null)
[ "${#PORTAL_CODE}" -gt 10 ] && ok "OptimismPortal present on Sepolia L1" || no "OptimismPortal has no code on L1"

# ---------------------------------------------------------------------------
hr "RESULT"
echo "PASS=$PASS  FAIL=$FAIL"
[ "$FAIL" -eq 0 ] && { echo "SMOKE TEST GREEN ✓"; exit 0; } || { echo "SMOKE TEST RED ✗"; exit 1; }
