#!/usr/bin/env bash
# Batch B — keychain + AA session-key depth (TEST_PLAN F3.3/3.4/3.8, F4.3, S1.1/1.3/1.6).
# root authorizes a session key (T3 KeyRestrictions, native-SCI limit); a session-key-signed
# sponsored 0x76 (root == fee_payer == keychain account) is:
#   within limit  -> executed as root (+credit)        over limit    -> rejected
#   after revoke  -> rejected                          CB-tripped    -> rejected; untrip -> ok
# All reads are poll-based (the validator EL settles state a beat after a tx mines).
# Run ON the deploy host. Public dev keys + freshly-generated session keys.
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

L2=${L2:-http://localhost:8545}
TXGEN=${TXGEN:-$HOME/sci-dev/sci-chain/target/release/sci-aa-txgen}
CHAIN_ID=42001
KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
SCISTATE=0xAAAAAAAA00000000000000000000000000000001
CB=0xBBBBBBBB00000000000000000000000000000003
ROOT=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266
ROOT_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DEST=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
ZERO=0x0000000000000000000000000000000000000000
AUTHZ='authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))'
LIMIT=50000000000000000; OKVAL=10000000000000000; OVERVAL=100000000000000000

PASS=0; FAIL=0
ok(){ echo "  PASS: $1"; PASS=$((PASS+1)); }
no(){ echo "  FAIL: $1"; FAIL=$((FAIL+1)); }
# root tx: cast send waits for the receipt; parse status robustly, then let state settle.
rootsend(){ local st; st=$(cast send "$@" --rpc-url "$L2" --private-key "$ROOT_KEY" --json 2>/dev/null | grep -oE '"status":"0x[01]"' | head -1); [ "$st" = '"status":"0x1"' ] || echo "    (warn: root tx not 0x1 for $1)"; sleep 3; }
authorize(){ rootsend $KEYCHAIN "$AUTHZ" "$1" 0 "(9999999999,true,[($ZERO,$LIMIT,0)],true,[])"; }
nonce(){ cast nonce "$1" --rpc-url "$L2" 2>/dev/null; }
bal(){ cast balance "$1" --rpc-url "$L2" 2>/dev/null; }
gk(){ cast call $KEYCHAIN 'getKey(address,address)(uint8,address,uint64,bool,bool)' $ROOT "$1" --rpc-url "$L2" 2>/dev/null | sed -n "${2}p"; }
wait_key(){ for _ in $(seq 1 20); do [ "$(gk "$1" 2 | tr A-Z a-z)" = "${1,,}" ] && return 0; sleep 2; done; return 1; }
wait_revoked(){ for _ in $(seq 1 15); do [ "$(gk "$1" 5)" = "true" ] && return 0; sleep 2; done; return 1; }
wait_tripped(){ for _ in $(seq 1 15); do [ "$(cast call $SCISTATE 'isTripped(address)(bool)' "$1" --rpc-url $L2 2>/dev/null)" = "$2" ] && return 0; sleep 2; done; return 1; }
wait_credit(){ for _ in $(seq 1 12); do python3 -c "exit(0 if int('$(bal "$1")')-int('$2')==$3 else 1)" 2>/dev/null && return 0; sleep 2; done; return 1; }
send_aa(){ # $1 signerKey $2 nonce $3 to $4 value -> MINED <st> <h> | POOLREJECT | NOSHOW <h> | GENFAIL
  local raw h st
  raw=$(GAS_LIMIT=300000 ROOT=$ROOT FEE_PAYER=$ROOT "$TXGEN" "$1" "$CHAIN_ID" "$2" "$3" "$4" 2>/dev/null | grep -oE '0x[0-9a-fA-F]{120,}' | tail -1)
  [ -z "$raw" ] && { echo GENFAIL; return; }
  h=$(cast rpc eth_sendRawTransaction "$raw" --rpc-url "$L2" 2>/dev/null | tr -d '"')
  { [ -z "$h" ] || [ "${h:0:2}" != "0x" ]; } && { echo POOLREJECT; return; }
  for _ in $(seq 1 12); do
    st=$(cast rpc eth_getTransactionReceipt "$h" --rpc-url "$L2" 2>/dev/null | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d.get("status","") if d else "")' 2>/dev/null)
    [ -n "$st" ] && { echo "MINED $st $h"; return; }
    sleep 2
  done; echo "NOSHOW $h"; }

echo "== Batch B: keychain + AA session-key depth =="
SK=$(cast wallet new 2>/dev/null); SK_ADDR=$(echo "$SK"|awk '/Address/{print $2}'); SK_KEY=$(echo "$SK"|awk '/Private key/{print $3}')
echo "  session key = $SK_ADDR"

echo "== F3.3 authorizeKey (T3, native-SCI limit) -> getKey =="
authorize "$SK_ADDR"; wait_key "$SK_ADDR" || echo "    (warn: key not visible)"
KID=$(gk "$SK_ADDR" 2); REV=$(gk "$SK_ADDR" 5)
[ "${KID,,}" = "${SK_ADDR,,}" ] && [ "$REV" = "false" ] && ok "key authorized (keyId matches, not revoked)" || no "authorize/getKey wrong (kid=$KID rev=$REV)"

echo "== F4.3 within-limit sponsored session-key 0x76 (expect MINED 0x1 + credit) =="
B0=$(bal $DEST); R=$(send_aa "$SK_KEY" "$(nonce $SK_ADDR)" "$DEST" "$OKVAL"); echo "  -> $R"
case "$R" in "MINED 0x1"*) ok "within-limit sponsored AA executed (status 1, ran as root)";; *) no "within-limit AA failed ($R)";; esac
wait_credit $DEST "$B0" $OKVAL && ok "recipient credited +0.01 (value moved from root)" || no "credit not observed"

echo "== F3.7/S1.3 over-limit sponsored AA (expect rejected, no credit) =="
B0=$(bal $DEST); R=$(send_aa "$SK_KEY" "$(nonce $SK_ADDR)" "$DEST" "$OVERVAL"); echo "  -> $R"
case "$R" in "MINED 0x1"*) no "over-limit AA executed — limit not enforced!";; *) ok "over-limit AA rejected ($R)";; esac
sleep 6; [ "$(bal $DEST)" = "$B0" ] && ok "recipient not credited over-limit" || no "credited despite over-limit"

echo "== F3.4/S1.1 revokeKey -> session-key AA rejected =="
rootsend $KEYCHAIN 'revokeKey(address)' $SK_ADDR
wait_revoked "$SK_ADDR" && ok "getKey shows revoked" || no "revoke not reflected"
R=$(send_aa "$SK_KEY" "$(nonce $SK_ADDR)" "$DEST" "$OKVAL"); echo "  -> $R"
case "$R" in "MINED 0x1"*) no "revoked-key AA executed!";; *) ok "revoked-key AA rejected ($R)";; esac

echo "== F3.8/S1.6 CircuitBreaker trip -> AA rejected -> untrip -> ok =="
SK2=$(cast wallet new 2>/dev/null); SK2_ADDR=$(echo "$SK2"|awk '/Address/{print $2}'); SK2_KEY=$(echo "$SK2"|awk '/Private key/{print $3}')
authorize "$SK2_ADDR"; wait_key "$SK2_ADDR" || echo "    (warn: SK2 not visible)"
rootsend $CB 'trip(address,bytes32)' $SK2_ADDR 0x0000000000000000000000000000000000000000000000000000000000000001
wait_tripped "$SK2_ADDR" true && ok "isTripped(SK2)=true after CB.trip" || no "trip did not set isTripped"
R=$(send_aa "$SK2_KEY" "$(nonce $SK2_ADDR)" "$DEST" "$OKVAL"); echo "  tripped -> $R"
case "$R" in "MINED 0x1"*) no "tripped-key AA executed!";; *) ok "tripped-key AA rejected ($R)";; esac
rootsend $CB 'untrip(address)' $SK2_ADDR
wait_tripped "$SK2_ADDR" false && ok "isTripped(SK2)=false after untrip" || no "untrip failed"
B0=$(bal $DEST); R=$(send_aa "$SK2_KEY" "$(nonce $SK2_ADDR)" "$DEST" "$OKVAL"); echo "  post-untrip -> $R"
{ case "$R" in "MINED 0x1"*) true;; *) false;; esac; } && wait_credit $DEST "$B0" $OKVAL \
  && ok "post-untrip AA executes again (+0.01)" || no "post-untrip AA did not execute ($R)"

echo "== RESULT =="; echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ] && { echo "BATCH B GREEN ✓"; exit 0; } || { echo "BATCH B RED ✗"; exit 1; }
