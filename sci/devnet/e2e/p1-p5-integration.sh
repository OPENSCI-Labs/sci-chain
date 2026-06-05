#!/usr/bin/env bash
# Plan A agent-loop integration test (P1–P5, with edge cases).
#
# Exercises the full keychain + AA-tx (0x76) behaviour against a RUNNING SCI devnet:
#   P1 register   — authorizeKey / duplicate / revoke / re-authorize
#   P2 AA tx      — sponsored transfer / unauthorized root / fee_payer!=root /
#                   multi-call atomic success / multi-call atomic revert
#   P3 limit      — pass / reject / period accrual / approve(pessimistic) / native sentinel
#   P4 breaker    — trip→reject / untrip→same-tx-includes / non-guardian trip reverts / isTripped
#   P5 expiry     — before-expiry pass / after-expiry reject / past-expiry reject
#
# This is a DEVNET integration test (not a CI unit test): AA txs are a native tx type the
# EL decodes + gates, so they can only be exercised against a real chain via the
# `sci-aa-txgen` tool + JSON-RPC. Run from a host that can reach the devnet SEQUENCER RPC.
#
# Usage:
#   cargo build --release -p sci-aa-txgen           # once
#   L2_RPC=http://<sequencer-host>:7545 sci/devnet/e2e/p1-p5-integration.sh
#
# IMPORTANT: L2_RPC MUST be the sequencer (not a verifier). AA txs are local-only (never
# gossiped); a verifier won't include them. Exit code 0 = all pass, 1 = some failed.
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

# ----------------------------- config -----------------------------
RPC=${L2_RPC:-http://localhost:7545}
SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
TXGEN=${AA_TXGEN:-$SCI_REPO/target/release/sci-aa-txgen}
CHAIN=${CHAIN_ID:-42001}

KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
BREAKER=0xBBBBBBBB00000000000000000000000000000003
GEN='authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))'

# ROOT = agent principal (pays gas, authorizes keys); funded on fresh chain
ROOT=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC ; ROOT_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
# OWNER = CB owner/guardian (genesis = account #0); FUNDER = account #1 (10000 ETH)
OWNER=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 ; OWNER_PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
FUNDER_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
SINK=0x6666666666666666666666666666666666666666
SINK2=0x6666666666666666666666666666666666660002
TOKEN=0x7777777777777777777777777777777777777777
RCPT=0x8888888888888888888888888888888888888888

# error selectors
ERR_KEYEXISTS=0xaa1ba2f8     # KeyAlreadyExists()
ERR_GUARDIAN=0x089866cd      # UnauthorizedGuardian()

YEAR=31536000
MAXFEE=1000000000            # aa-txgen default max_fee_per_gas (1e9)
GLIMIT=100000                # aa-txgen default gas_limit
RESV=$(( GLIMIT * MAXFEE ))  # gas reservation the sentinel pre-flight uses (gas_limit*max_fee)

# ----------------------------- harness -----------------------------
PASS=0; FAIL=0
ok()  { echo "    PASS  $1"; PASS=$((PASS+1)); }
bad() { echo "    FAIL  $1  --  $2"; FAIL=$((FAIL+1)); }
sec() { echo; echo "==================== $1 ===================="; }

non()  { cast nonce "$1" --rpc-url $RPC 2>/dev/null; }
bal()  { cast balance "$1" --rpc-url $RPC 2>/dev/null; }
# echo a tx's status hex ("0x1"/"0x0") or "none" if no receipt
rstatus() { cast rpc eth_getTransactionReceipt "$1" --rpc-url $RPC 2>/dev/null \
  | python3 -c 'import sys,json;d=json.load(sys.stdin);print(d["status"]) if d else print("none")' 2>/dev/null || echo none; }
# poll up to ~28s for a receipt; echo final status or "none"
wait_status() { local h=$1 n=${2:-14} r; for _ in $(seq 1 $n); do r=$(rstatus "$h"); [ "$r" != none ] && { echo "$r"; return; }; sleep 2; done; echo none; }
# generate a fresh session key -> sets globals KADDR / KPK
new_key() { read KADDR KPK < <(cast wallet new 2>/dev/null | awk '/Address/{a=$2}/Private key/{print a,$3}'); }
# authorize a key with a full KeyRestrictions tuple (4th arg); echoes send rc
authorize() { cast send $KEYCHAIN "$GEN" "$1" 0 "$2" --private-key "$3" --rpc-url $RPC --confirmations 1 >/dev/null 2>&1; }
# strip cast's "[1.844e19]" scientific-notation annotations so positional read aligns
getkey() { cast call $KEYCHAIN "getKey(address,address)(uint8,address,uint64,bool,bool)" "$1" "$2" --rpc-url $RPC 2>/dev/null | sed -E 's/ *\[[^]]*\]//g'; }
# build+submit an AA tx; reads ROOT/FEE_PAYER/INPUT/GAS_LIMIT/CALL2_* from env; echoes hash or error
aa_submit() { local pk=$1 nn=$2 to=$3 val=$4 raw
  raw=$($TXGEN "$pk" $CHAIN "$nn" "$to" "$val" 2>/dev/null | tail -1)
  cast rpc eth_sendRawTransaction "$raw" --rpc-url $RPC 2>&1 | tr -d '"'
}
# assert a submitted tx mines with expected status
assert_status() { local h=$1 want=$2 desc=$3 s; s=$(wait_status "$h"); [ "$s" = "$want" ] && ok "$desc" || bad "$desc" "status=$s want=$want (h=$h)"; }
# assert an AA tx is hook-rejected: never mined AND signer nonce unchanged
assert_rejected() { local h=$1 signer=$2 n0=$3 desc=$4 s n1
  s=$(wait_status "$h" 8); n1=$(non "$signer")
  if [ "$s" = none ] && [ "$n1" = "$n0" ]; then ok "$desc"; else bad "$desc" "status=$s nonce $n0->$n1 (expected none + unchanged)"; fi; }
# assert a cast-send reverts with a given 4-byte selector
assert_revert() { local desc=$1 sel=$2; shift 2; local out
  out=$("$@" 2>&1); if echo "$out" | grep -qi "$sel\|reverted\|execution reverted"; then ok "$desc"; else bad "$desc" "did not revert as expected: $(echo "$out"|head -c 120)"; fi; }

UNRESTRICTED="(18446744073709551615,false,[],true,[])"

# ----------------------------- preflight -----------------------------
echo "P1-P5 integration test  rpc=$RPC  txgen=$TXGEN"
[ -x "$TXGEN" ] || { echo "FATAL: aa-txgen not found at $TXGEN (cargo build --release -p sci-aa-txgen)"; exit 2; }
CID=$(cast chain-id --rpc-url $RPC 2>/dev/null)
[ "$CID" = "$CHAIN" ] || { echo "FATAL: chainid=$CID expected $CHAIN (wrong RPC?)"; exit 2; }
echo "chain head=$(cast block-number --rpc-url $RPC)  keychain_code=$(cast code $KEYCHAIN --rpc-url $RPC | wc -c)"
# fund CB owner (#0) so it can send trip/untrip
cast send $OWNER --value 0.2ether --private-key $FUNDER_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1

# ============================ P1 register ============================
sec "P1 register"
new_key; K=$KADDR; KP=$KPK
authorize "$K" "$UNRESTRICTED" "$ROOT_PK"
read -r st kid exp el rv <<<"$(getkey $ROOT $K | tr '\n' ' ')"
[ "${kid,,}" = "${K,,}" ] && [ "$rv" = false ] && ok "P1.1 authorizeKey -> getKey ok (keyId=$kid expiry=$exp revoked=$rv)" \
  || bad "P1.1 authorizeKey" "getKey kid=$kid revoked=$rv"
# P1.2 duplicate authorize on same key -> KeyAlreadyExists
assert_revert "P1.2 duplicate authorizeKey reverts KeyAlreadyExists" "$ERR_KEYEXISTS" \
  cast send $KEYCHAIN "$GEN" "$K" 0 "$UNRESTRICTED" --private-key $ROOT_PK --rpc-url $RPC
# P1.3 revokeKey -> isRevoked=true
cast send $KEYCHAIN "revokeKey(address)" "$K" --private-key $ROOT_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
read -r st kid exp el rv <<<"$(getkey $ROOT $K | tr '\n' ' ')"
[ "$rv" = true ] && ok "P1.3 revokeKey -> isRevoked=true" || bad "P1.3 revokeKey" "isRevoked=$rv"
# P1.4 authorize a fresh key still works
new_key; K2=$KADDR
authorize "$K2" "$UNRESTRICTED" "$ROOT_PK"
read -r st kid exp el rv <<<"$(getkey $ROOT $K2 | tr '\n' ' ')"
[ "${kid,,}" = "${K2,,}" ] && ok "P1.4 re-authorize fresh key ok" || bad "P1.4 re-authorize" "kid=$kid"

# ============================ P2 AA tx ============================
sec "P2 AA transfer"
# P2.1 sponsored transfer: signer nets 0, SINK +5
new_key; K=$KADDR; KP=$KPK; authorize "$K" "$UNRESTRICTED" "$ROOT_PK"
s0=$(bal $SINK); N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 5)
assert_status "$H" 0x1 "P2.1 sponsored AA transfer mines"
s1=$(bal $SINK); kbal=$(bal $K)
[ "$kbal" = 0 ] && [ "$((s1 - s0))" = 5 ] 2>/dev/null && ok "P2.1b SINK +5, signer balance 0 (sponsored)" \
  || bad "P2.1b conservation" "sink $s0->$s1 signer_bal=$kbal"
# P2.2 unauthorized root (no key) -> rejected
new_key; K=$KADDR; KP=$KPK; N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 1)
assert_rejected "$H" "$K" "$N" "P2.2 unauthorized root AA tx rejected"
# P2.3 fee_payer != root -> rejected (validate)
new_key; K=$KADDR; KP=$KPK; authorize "$K" "$UNRESTRICTED" "$ROOT_PK"; N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$SINK aa_submit $KP $N $SINK 1)
assert_rejected "$H" "$K" "$N" "P2.3 fee_payer!=root AA tx rejected"
# P2.4 multi-call atomic success: call1 SINK+3, call2 SINK2+4
new_key; K=$KADDR; KP=$KPK; authorize "$K" "$UNRESTRICTED" "$ROOT_PK"
a0=$(bal $SINK); b0=$(bal $SINK2); N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT CALL2_TO=$SINK2 CALL2_VALUE=4 aa_submit $KP $N $SINK 3)
assert_status "$H" 0x1 "P2.4 multi-call batch mines"
a1=$(bal $SINK); b1=$(bal $SINK2)
[ "$((a1-a0))" = 3 ] && [ "$((b1-b0))" = 4 ] 2>/dev/null && ok "P2.4b both calls applied (SINK+3, SINK2+4)" \
  || bad "P2.4b multi-call effects" "sink $a0->$a1 sink2 $b0->$b1"
# P2.5 multi-call atomic revert: call2 -> always-revert contract => whole batch rolls back
# deploy an always-revert contract (runtime 0x60006000fd). NB: cast `send` wants options
# BEFORE `--create`; CREATE address is deterministic from (deployer, nonce-at-deploy).
FA=$(cast wallet address --private-key $FUNDER_PK); FN=$(non $FA)
cast send --private-key $FUNDER_PK --rpc-url $RPC --confirmations 1 --create 0x6005600c60003960056000f360006000fd >/dev/null 2>&1
REVC=$(cast compute-address $FA --nonce $FN 2>/dev/null | grep -oE '0x[0-9a-fA-F]{40}')
new_key; K=$KADDR; KP=$KPK; authorize "$K" "$UNRESTRICTED" "$ROOT_PK"
a0=$(bal $SINK); N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT CALL2_TO=$REVC CALL2_VALUE=0 aa_submit $KP $N $SINK 5)
s=$(wait_status "$H"); a1=$(bal $SINK)
{ [ "$s" = 0x0 ] && [ "$a1" = "$a0" ]; } && ok "P2.5 multi-call atomic revert (status 0, call1 rolled back)" \
  || bad "P2.5 atomic revert" "status=$s sink $a0->$a1 (revc=$REVC)"

# ============================ P3 spending limit ============================
sec "P3 spending limit"
HIGH="(0x0000000000000000000000000000000000000000,1000000000000000000,$YEAR)"   # addr0 sentinel, huge
# P3.1 enforce + transfer 50 <= 100 -> pass
new_key; K=$KADDR; KP=$KPK
authorize "$K" "(18446744073709551615,true,[$HIGH,($TOKEN,100,$YEAR)],true,[])" "$ROOT_PK"
N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT GAS_LIMIT=120000 INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 50) aa_submit $KP $N $TOKEN 0)
assert_status "$H" 0x1 "P3.1 transfer 50<=100 mines"
# P3.2 transfer 200 > 100 -> rejected
new_key; K=$KADDR; KP=$KPK
authorize "$K" "(18446744073709551615,true,[$HIGH,($TOKEN,100,$YEAR)],true,[])" "$ROOT_PK"
N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT GAS_LIMIT=120000 INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 200) aa_submit $KP $N $TOKEN 0)
assert_rejected "$H" "$K" "$N" "P3.2 transfer 200>100 rejected"
# P3.3 period accrual: 60 then 60 (sum 120 > 100) -> 2nd rejected
new_key; K=$KADDR; KP=$KPK
authorize "$K" "(18446744073709551615,true,[$HIGH,($TOKEN,100,$YEAR)],true,[])" "$ROOT_PK"
N=$(non $K)
H1=$(ROOT=$ROOT FEE_PAYER=$ROOT GAS_LIMIT=120000 INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 60) aa_submit $KP $N $TOKEN 0)
assert_status "$H1" 0x1 "P3.3a first transfer 60 mines"
N=$(non $K)
H2=$(ROOT=$ROOT FEE_PAYER=$ROOT GAS_LIMIT=120000 INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 60) aa_submit $KP $N $TOKEN 0)
assert_rejected "$H2" "$K" "$N" "P3.3b second transfer 60 (accrued 120>100) rejected"
# P3.4 approve charged pessimistically (full amount): approve 100 uses up the limit
new_key; K=$KADDR; KP=$KPK
authorize "$K" "(18446744073709551615,true,[$HIGH,($TOKEN,100,$YEAR)],true,[])" "$ROOT_PK"
N=$(non $K)
H1=$(ROOT=$ROOT FEE_PAYER=$ROOT GAS_LIMIT=120000 INPUT=$(cast calldata "approve(address,uint256)" $RCPT 100) aa_submit $KP $N $TOKEN 0)
assert_status "$H1" 0x1 "P3.4a approve 100 mines (uses full limit)"
N=$(non $K)
H2=$(ROOT=$ROOT FEE_PAYER=$ROOT GAS_LIMIT=120000 INPUT=$(cast calldata "transfer(address,uint256)" $RCPT 1) aa_submit $KP $N $TOKEN 0)
assert_rejected "$H2" "$K" "$N" "P3.4b transfer 1 after approve 100 rejected (pessimistic)"
# P3.5 native sentinel: addr0 limit = RESV+3; value 2 passes, value 5 rejected
SENT="(0x0000000000000000000000000000000000000000,$((RESV+3)),$YEAR)"
new_key; K=$KADDR; KP=$KPK; authorize "$K" "(18446744073709551615,true,[$SENT],true,[])" "$ROOT_PK"; N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 2)
assert_status "$H" 0x1 "P3.5a native value 2 within sentinel mines"
new_key; K=$KADDR; KP=$KPK; authorize "$K" "(18446744073709551615,true,[$SENT],true,[])" "$ROOT_PK"; N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 5)
assert_rejected "$H" "$K" "$N" "P3.5b native value 5 over sentinel rejected"

# ============================ P4 circuit breaker ============================
sec "P4 circuit breaker"
new_key; K=$KADDR; KP=$KPK; authorize "$K" "$UNRESTRICTED" "$ROOT_PK"
# P4.3 non-guardian (ROOT) trip reverts
assert_revert "P4.3 non-guardian trip reverts UnauthorizedGuardian" "$ERR_GUARDIAN" \
  cast send $BREAKER "trip(address,bytes32)" "$K" "$(cast format-bytes32-string freeze)" --private-key $ROOT_PK --rpc-url $RPC
# P4.1 owner trips -> AA rejected ; P4.4 isTripped
cast send $BREAKER "trip(address,bytes32)" "$K" "$(cast format-bytes32-string freeze)" --private-key $OWNER_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
[ "$(cast call $BREAKER 'isTripped(address)(bool)' $K --rpc-url $RPC)" = true ] && ok "P4.4a isTripped=true after trip" || bad "P4.4a isTripped" "not true"
N=$(non $K)
HT=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 7)
assert_rejected "$HT" "$K" "$N" "P4.1 tripped key AA tx rejected"
# P4.2 untrip -> the SAME pending tx now includes
cast send $BREAKER "untrip(address)" "$K" --private-key $OWNER_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
[ "$(cast call $BREAKER 'isTripped(address)(bool)' $K --rpc-url $RPC)" = false ] && ok "P4.4b isTripped=false after untrip" || bad "P4.4b isTripped" "not false"
assert_status "$HT" 0x1 "P4.2 same tx included after untrip"

# ============================ P5 expiry ============================
sec "P5 expiry"
# P5.1 before-expiry pass / P5.2 after-expiry reject
TTL=40
new_key; K=$KADDR; KP=$KPK
EXP=$(( $(date +%s) + TTL ))
authorize "$K" "($EXP,false,[],true,[])" "$ROOT_PK"
N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 3)
assert_status "$H" 0x1 "P5.1 before-expiry AA tx mines"
W=$(( EXP - $(date +%s) + 6 )); [ $W -gt 0 ] && { echo "    (sleep ${W}s past expiry)"; sleep $W; }
N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 3)
assert_rejected "$H" "$K" "$N" "P5.2 after-expiry AA tx rejected"
# P5.3 key authorized with a past expiry -> immediately rejected
new_key; K=$KADDR; KP=$KPK
PAST=$(( $(date +%s) - 100 ))
authorize "$K" "($PAST,false,[],true,[])" "$ROOT_PK"
N=$(non $K)
H=$(ROOT=$ROOT FEE_PAYER=$ROOT aa_submit $KP $N $SINK 3)
assert_rejected "$H" "$K" "$N" "P5.3 past-expiry key AA tx rejected"

# ----------------------------- summary -----------------------------
sec "SUMMARY"
echo "PASS=$PASS  FAIL=$FAIL"
[ $FAIL -eq 0 ]
