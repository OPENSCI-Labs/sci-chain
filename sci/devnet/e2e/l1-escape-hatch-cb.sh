#!/usr/bin/env bash
# Tier 1 L1 forced-inclusion escape hatch e2e — emergency CircuitBreaker freeze via L1.
#
# Proves a censored owner can still halt (and resume) an agent by force-including an
# AgentCircuitBreaker.trip/untrip through the L1 OptimismPortal deposit path, and that the
# onlyGuardian authorization holds even via L1 (a non-owner cannot force-trip). See the design
# in sci/docs/plan-a-l1-escape-hatch.md (Tier 1).
#
# Mechanism: an L1 EOA's depositTransaction lands on L2 with msg.sender == that EOA (OP-Stack
# portal passes EOAs through unaliased). AgentCircuitBreaker (0xBBBB..03) authorizes on
# msg.sender (Ownable onlyGuardian), so a deposit from the owner/guardian trips the breaker.
# No SCI core code change is involved — Tier 1 composes existing primitives.
#
# Run on the devnet host (needs foundry `cast`, `jq`, `python3`, and a running devnet).
# Config (env overrides):
#   L1_RPC      L1 EL RPC                  (default http://localhost:4545)
#   L2_RPC      L2 EL RPC                  (default http://localhost:8545)
#   OWNER_PK    private key of the CB owner/guardian (default hardhat #0 = ACC0)
#   NONOWNER_PK private key NOT owner/guardian        (default hardhat #1 = ACC1)
#   DEPOSIT_GAS L2 gas limit for the forced call      (default 250000)
#   POLL_SECS   max seconds to wait for a deposit to derive (default 180)
set -uo pipefail
export PATH=$PATH:~/.foundry/bin:~/.cargo/bin

SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}
cd "$SCI_REPO"
L1=${L1_RPC:-http://localhost:4545}
L2=${L2_RPC:-http://localhost:8545}
OWNER_PK=${OWNER_PK:-0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80}
NONOWNER_PK=${NONOWNER_PK:-0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d}
DEPOSIT_GAS=${DEPOSIT_GAS:-250000}
POLL_SECS=${POLL_SECS:-180}

CB=0xBBBBBBBB00000000000000000000000000000003
# Session keys used purely as trip targets (any address works; isTripped is a bare mapping).
SESSION_KEY=${SESSION_KEY:-0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC}   # ACC2 (freeze/unfreeze)
NEG_KEY=${NEG_KEY:-0x90F79bf6EB2c4f870365E785982E1f101E93b906}           # ACC3 (negative control)
MARKER_KEY=${MARKER_KEY:-0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65}     # ACC4 (derivation marker)
REASON=$(cast format-bytes32-string "l1-escape-hatch")

PORTAL=$(jq -r '.OptimismPortalProxy' .devnet/l2/configs/l1-addresses.json 2>/dev/null)
[ -z "$PORTAL" -o "$PORTAL" = null ] && { echo "FAIL: could not read OptimismPortalProxy from .devnet/l2/configs/l1-addresses.json"; exit 2; }
OWNER_ADDR=$(cast wallet address --private-key "$OWNER_PK")
CB_OWNER=$(cast call $CB 'owner()(address)' --rpc-url $L2)
echo "portal=$PORTAL  CB=$CB  CB.owner()=$CB_OWNER  OWNER_PK addr=$OWNER_ADDR"
[ "${CB_OWNER,,}" = "${OWNER_ADDR,,}" ] || echo "WARN: OWNER_PK ($OWNER_ADDR) is not CB.owner() ($CB_OWNER); ensure it is owner OR a guardian, else trips will revert."

tripped(){ cast call $CB 'isTripped(address)(bool)' "$1" --rpc-url $L2 2>/dev/null; }
deposit(){ # <calldata> <pk> ; force-include CB.<call> from L1
  cast send $PORTAL 'depositTransaction(address,uint256,uint64,bool,bytes)' $CB 0 $DEPOSIT_GAS false "$1" \
    --private-key "$2" --rpc-url $L1 --json 2>/dev/null \
    | python3 -c "import sys,json;print(json.load(sys.stdin).get('transactionHash','?'))" 2>/dev/null
}
# wait_until <addr> <want true|false>
wait_until(){ local addr=$1 want=$2 i;
  for ((i=10; i<=POLL_SECS; i+=10)); do
    [ "$(tripped "$addr")" = "$want" ] && { echo "    derived after ~${i}s"; return 0; }
    sleep 10
  done
  return 1
}

FAIL=0
echo
echo "=== A: owner force-FREEZE via L1 (expect isTripped -> true) ==="
[ "$(tripped $SESSION_KEY)" = "true" ] && { echo "  pre-state tripped; untripping first"; deposit "$(cast calldata 'untrip(address)' $SESSION_KEY)" "$OWNER_PK" >/dev/null; wait_until $SESSION_KEY false || true; }
echo "  L1 trip tx: $(deposit "$(cast calldata 'trip(address,bytes32)' $SESSION_KEY $REASON)" "$OWNER_PK")"
if wait_until $SESSION_KEY true; then echo "  PASS: session key FROZEN via L1 forced inclusion"; else echo "  FAIL: not frozen within ${POLL_SECS}s"; FAIL=1; fi

echo
echo "=== B: owner force-UNFREEZE via L1 (expect isTripped -> false) ==="
echo "  L1 untrip tx: $(deposit "$(cast calldata 'untrip(address)' $SESSION_KEY)" "$OWNER_PK")"
if wait_until $SESSION_KEY false; then echo "  PASS: session key UNFROZEN via L1"; else echo "  FAIL: still frozen within ${POLL_SECS}s"; FAIL=1; fi

echo
echo "=== C: NEGATIVE — non-owner force-trip must NOT freeze (auth holds via L1) ==="
echo "  L1 non-owner trip(NEG_KEY) tx: $(deposit "$(cast calldata 'trip(address,bytes32)' $NEG_KEY $REASON)" "$NONOWNER_PK")"
echo "  L1 owner trip(MARKER) tx (derivation marker): $(deposit "$(cast calldata 'trip(address,bytes32)' $MARKER_KEY $REASON)" "$OWNER_PK")"
if wait_until $MARKER_KEY true; then
  if [ "$(tripped $NEG_KEY)" = "false" ]; then echo "  PASS: non-owner trip rejected (NEG_KEY still false after marker derived)"; else echo "  FAIL: non-owner managed to trip NEG_KEY"; FAIL=1; fi
else
  echo "  FAIL: marker did not derive; negative control inconclusive"; FAIL=1
fi
echo "  cleanup: untrip(MARKER) tx: $(deposit "$(cast calldata 'untrip(address)' $MARKER_KEY)" "$OWNER_PK")"
wait_until $MARKER_KEY false || true

echo
[ $FAIL -eq 0 ] && echo "=== RESULT: PASS — Tier 1 L1 escape hatch (freeze/unfreeze; non-owner rejected) ===" || echo "=== RESULT: FAIL ==="
exit $FAIL
