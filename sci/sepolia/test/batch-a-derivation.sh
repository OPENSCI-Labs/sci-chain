#!/usr/bin/env bash
# Batch A — derivation robustness (TEST_PLAN F1.2 / F1.3).
# Proves the validator reconstructs the exact same chain as the sequencer.
#   default:  multi-height hash+stateRoot consistency (non-disruptive)
#   --deep :  stop sequencer op-node, prove validator SAFE head still climbs to the
#             last-produced block purely from L1 calldata, then restart it (F1.3)
# Run ON the deploy host. Endpoints env-overridable. Exits non-zero on any failure.
set -uo pipefail
export PATH=$PATH:~/.foundry/bin
SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." 2>/dev/null && pwd)}

L2_VAL=${L2_VAL:-http://localhost:8545}   # validator EL
L2_SEQ=${L2_SEQ:-http://localhost:7545}   # sequencer EL
OPNODE_SEQ=${OPNODE_SEQ:-http://localhost:7549}
OPNODE_VAL=${OPNODE_VAL:-http://localhost:8549}
SEQ_CL=${SEQ_CL:-base-builder-cl}         # sequencer op-node container
COMPOSE="docker compose --env-file etc/docker/sepolia-runtime-env -f etc/docker/docker-compose.sepolia.yml -f sci/devnet/docker-compose.sci.yml -f etc/docker/docker-compose.sepolia-hosts.yml"
DEEP=0; [ "${1:-}" = "--deep" ] && DEEP=1

PASS=0; FAIL=0
ok(){ echo "  PASS: $1"; PASS=$((PASS+1)); }
no(){ echo "  FAIL: $1"; FAIL=$((FAIL+1)); }
ss(){ curl -s "$1" -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"optimism_syncStatus","params":[]}' 2>/dev/null; }
safe_of(){ ss "$1" | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["safe_l2"]["number"])' 2>/dev/null; }
unsafe_of(){ ss "$1" | python3 -c 'import sys,json;print(json.load(sys.stdin)["result"]["unsafe_l2"]["number"])' 2>/dev/null; }
hash_at(){ cast block "$2" --rpc-url "$1" --field hash 2>/dev/null; }
root_at(){ cast block "$2" --rpc-url "$1" --field stateRoot 2>/dev/null; }

echo "== Batch A: derivation robustness =="
SEQ_UNSAFE=$(unsafe_of "$OPNODE_SEQ"); VAL_SAFE=$(safe_of "$OPNODE_VAL")
echo "  sequencer unsafe=$SEQ_UNSAFE | validator safe=$VAL_SAFE"
[ -n "$VAL_SAFE" ] && [ "$VAL_SAFE" -gt 0 ] && ok "validator safe head > 0 (L1-derived)" || no "validator safe head not advancing"

echo "== multi-height consistency (seq EL vs val EL) =="
HEIGHTS=$([ "${VAL_SAFE:-0}" -gt 30 ] && echo "10 $((VAL_SAFE/2)) $((VAL_SAFE-5))" || echo "1 2 3")
for H in $HEIGHTS; do
  HS=$(hash_at "$L2_SEQ" "$H"); HV=$(hash_at "$L2_VAL" "$H")
  RS=$(root_at "$L2_SEQ" "$H"); RV=$(root_at "$L2_VAL" "$H")
  if [ -n "$HS" ] && [ "$HS" = "$HV" ] && [ "$RS" = "$RV" ]; then ok "block $H: hash+stateRoot identical on seq/val"
  else no "block $H mismatch (seq $HS/$RS vs val $HV/$RV)"; fi
done

if [ "$DEEP" = 1 ]; then
  echo "== DEEP: stop sequencer op-node, prove L1-only derivation catches up =="
  [ -d "$SCI_REPO" ] || { no "SCI_REPO not found ($SCI_REPO) — cannot run deep mode"; }
  cd "$SCI_REPO" || true
  TARGET=$(unsafe_of "$OPNODE_SEQ")
  echo "  target (last unsafe before stop) = $TARGET"
  echo "  stopping $SEQ_CL ..."
  docker stop "$SEQ_CL" >/dev/null 2>&1
  trap 'echo "  [trap] restarting $SEQ_CL"; $COMPOSE up -d --no-deps "$SEQ_CL" >/dev/null 2>&1' EXIT
  CAUGHT=0
  for i in $(seq 1 24); do
    VS=$(safe_of "$OPNODE_VAL"); echo "  [$((i*10))s] validator safe=$VS (target $TARGET, sequencer stopped)"
    if [ -n "$VS" ] && [ "$VS" -ge "$TARGET" ]; then CAUGHT=1; break; fi
    sleep 10
  done
  if [ "$CAUGHT" = 1 ]; then
    ok "validator safe reached $TARGET via L1 calldata ONLY (sequencer down)"
    HV=$(hash_at "$L2_VAL" "$TARGET"); HS=$(hash_at "$L2_SEQ" "$TARGET")
    [ -n "$HV" ] && [ "$HV" = "$HS" ] && ok "L1-derived block $TARGET hash matches sequencer ($HV)" || no "derived block $TARGET hash mismatch"
  else
    no "validator did not reach $TARGET within window (batches may lag; inspect)"
  fi
  echo "  restarting $SEQ_CL ..."
  $COMPOSE up -d --no-deps "$SEQ_CL" >/dev/null 2>&1
  trap - EXIT
  for i in $(seq 1 20); do
    U2=$(unsafe_of "$OPNODE_SEQ"); [ -n "$U2" ] && [ "$U2" -gt "$TARGET" ] && break; sleep 3
  done
  [ -n "${U2:-}" ] && [ "${U2:-0}" -gt "$TARGET" ] && ok "block production resumed (unsafe $TARGET -> $U2)" || no "production did NOT resume — CHECK $SEQ_CL"
fi

echo "== RESULT =="; echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ] && { echo "BATCH A GREEN ✓"; exit 0; } || { echo "BATCH A RED ✗"; exit 1; }
