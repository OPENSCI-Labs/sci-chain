#!/usr/bin/env bash
# Plan A Option-B agent closed-loop e2e (native AA tx 0x76, no EIP-7702).
#
# Drives the full agent loop against a running SCI devnet:
#   P1 register   — ROOT authorizes a session key on the keychain (no 7702)
#   P2 transfer   — session key signs a 0x76 AA tx, fee_payer==root sponsors gas
#   P3 limit      — enforced spending limit: positive (<=cap) mined / negative (>cap) rejected
#   P4 breaker    — circuit-breaker trip -> AA tx rejected ; untrip -> same tx included
#   P5 expiry     — key expiry: before-expiry mined / after-expiry rejected
#
# Spec + expected outputs: sci/docs/plan-a-aa-e2e.md (runbook).
# Devnet-verified 2026-06-04 (commit 25c485a92): full P1-P5 green, chain never wedged.
#
# Config (env overrides):
#   L2_RPC    devnet L2 RPC      (default http://localhost:8545; use the sequencer :7545
#                                 directly to bypass the verifier's sendRawTransaction proxy)
#   SCI_REPO  repo root          (default: inferred from this script's location)
#   AA_TXGEN  sci-aa-txgen path  (default: $SCI_REPO/target/release/sci-aa-txgen)
#   CHAIN_ID  L2 chain id        (default 42001)
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

RPC=${L2_RPC:-http://localhost:8545}
SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}
TXGEN=${AA_TXGEN:-$SCI_REPO/target/release/sci-aa-txgen}
CHAIN=${CHAIN_ID:-42001}

KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
BREAKER=0xBBBBBBBB00000000000000000000000000000003
GEN="authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))"

# Standard test mnemonic accounts ("test test ... junk").
ROOT=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC ; ROOT_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
ACC4=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65 ; ACC4_PK=0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a
ACC5=0x9965507D1a55bcC2695C58ba16FB37d819B0A4dc ; ACC5_PK=0x8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba
ACC3=0x90F79bf6EB2c4f870365E785982E1f101E93b906 ; ACC3_PK=0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6
ACC0=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 ; ACC0_PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
ACC1_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
SINK=0x6666666666666666666666666666666666666666
T=0x7777777777777777777777777777777777777777 ; RCPT=0x8888888888888888888888888888888888888888

rcpt(){ cast rpc eth_getTransactionReceipt "$1" --rpc-url $RPC 2>/dev/null | python3 -c 'import sys,json;d=json.load(sys.stdin);print("status="+d["status"]+" block="+str(int(d["blockNumber"],16))+" gas="+str(int(d["gasUsed"],16))) if d else print("none")' 2>/dev/null; }
# poll a hash up to N*4s; echo final state
wait_rcpt(){ local h=$1 n=${2:-10} r; for _ in $(seq 1 $n); do r=$(rcpt "$h"); [ "$r" != "none" ] && { echo "$r"; return; }; sleep 4; done; echo "none"; }
bal(){ cast balance "$1" --rpc-url $RPC 2>/dev/null; }
non(){ cast nonce "$1" --rpc-url $RPC 2>/dev/null; }

echo "===== FRESH CHAIN e2e (rpc=$RPC head=$(cast block-number --rpc-url $RPC)) ====="
echo "[setup] fund CB owner ACC0"
cast send $ACC0 --value 0.1ether --private-key $ACC1_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1 && echo "  ACC0 bal=$(bal $ACC0)"

echo "===== P1 register: ROOT authorizeKey(ACC4, unrestricted) ====="
cast send $KEYCHAIN "$GEN" $ACC4 0 "(18446744073709551615,false,[],true,[])" --private-key $ROOT_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
echo "  getKey ROOT->ACC4: $(cast call $KEYCHAIN 'getKey(address,address)(uint8,address,uint64,bool,bool)' $ROOT $ACC4 --rpc-url $RPC 2>&1 | tr '\n' ' ')"

echo "===== P2 AA transfer: ACC4 root=fp=ROOT value=5 -> SINK ====="
s0=$(bal $SINK)
RAW=$(ROOT=$ROOT FEE_PAYER=$ROOT $TXGEN $ACC4_PK $CHAIN $(non $ACC4) $SINK 5 2>/dev/null | tail -1)
H=$(cast rpc eth_sendRawTransaction "$RAW" --rpc-url $RPC 2>&1 | tr -d '"')
echo "  hash=$H  -> $(wait_rcpt $H 12)"
echo "  SINK $s0 -> $(bal $SINK) | ACC4(signer) bal=$(bal $ACC4) nonce=$(non $ACC4)"

echo "===== P3 limit: authorizeKey(ACC5, enforce, [addr0:1e18, T:100]) ====="
cast send $KEYCHAIN "$GEN" $ACC5 0 "(18446744073709551615,true,[(0x0000000000000000000000000000000000000000,1000000000000000000,31536000),(0x7777777777777777777777777777777777777777,100,31536000)],true,[])" --private-key $ROOT_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
echo "  getKey ROOT->ACC5: $(cast call $KEYCHAIN 'getKey(address,address)(uint8,address,uint64,bool,bool)' $ROOT $ACC5 --rpc-url $RPC 2>&1 | tr '\n' ' ')"
N5=$(non $ACC5)
IPOS=$(cast calldata "transfer(address,uint256)" $RCPT 50)
INEG=$(cast calldata "transfer(address,uint256)" $RCPT 200)
RP=$(ROOT=$ROOT FEE_PAYER=$ROOT INPUT=$IPOS GAS_LIMIT=120000 $TXGEN $ACC5_PK $CHAIN $N5 $T 0 2>/dev/null | tail -1)
HP=$(cast rpc eth_sendRawTransaction "$RP" --rpc-url $RPC 2>&1 | tr -d '"')
RN=$(ROOT=$ROOT FEE_PAYER=$ROOT INPUT=$INEG GAS_LIMIT=120000 $TXGEN $ACC5_PK $CHAIN $((N5+1)) $T 0 2>/dev/null | tail -1)
HN=$(cast rpc eth_sendRawTransaction "$RN" --rpc-url $RPC 2>&1 | tr -d '"')
echo "  pos(transfer 50<=100) $HP -> $(wait_rcpt $HP 12)"
echo "  neg(transfer 200>100) $HN -> $(wait_rcpt $HN 6) (expect none)"
echo "  ACC5 nonce=$(non $ACC5) (expect $((N5+1)) = pos mined, neg rejected)"

echo "===== P4 circuit breaker: trip ACC4 -> reject ; untrip -> include ====="
cast send $BREAKER "trip(address,bytes32)" $ACC4 $(cast format-bytes32-string "freeze") --private-key $ACC0_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
echo "  isTripped(ACC4)=$(cast call $BREAKER 'isTripped(address)(bool)' $ACC4 --rpc-url $RPC)"
RT=$(ROOT=$ROOT FEE_PAYER=$ROOT $TXGEN $ACC4_PK $CHAIN $(non $ACC4) $SINK 7 2>/dev/null | tail -1)
HT=$(cast rpc eth_sendRawTransaction "$RT" --rpc-url $RPC 2>&1 | tr -d '"')
echo "  tripped AA tx $HT -> $(wait_rcpt $HT 6) (expect none)"
cast send $BREAKER "untrip(address)" $ACC4 --private-key $ACC0_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
echo "  isTripped(ACC4)=$(cast call $BREAKER 'isTripped(address)(bool)' $ACC4 --rpc-url $RPC)"
echo "  same tx after untrip -> $(wait_rcpt $HT 12) (expect status=0x1)"
echo "  SINK bal=$(bal $SINK)"

echo "===== P5 expiry: authorizeKey(ACC3, expiry=now+45s) ; before pass / after reject ====="
EXP=$(( $(date +%s) + 45 ))
cast send $KEYCHAIN "$GEN" $ACC3 0 "($EXP,false,[],true,[])" --private-key $ROOT_PK --rpc-url $RPC --confirmations 1 >/dev/null 2>&1
echo "  getKey ROOT->ACC3 (expiry=$EXP): $(cast call $KEYCHAIN 'getKey(address,address)(uint8,address,uint64,bool,bool)' $ROOT $ACC3 --rpc-url $RPC 2>&1 | tr '\n' ' ')"
RB=$(ROOT=$ROOT FEE_PAYER=$ROOT $TXGEN $ACC3_PK $CHAIN $(non $ACC3) $SINK 3 2>/dev/null | tail -1)
HB=$(cast rpc eth_sendRawTransaction "$RB" --rpc-url $RPC 2>&1 | tr -d '"')
echo "  before-expiry AA tx $HB -> $(wait_rcpt $HB 10) (expect status=0x1)"
now=$(date +%s); [ $now -lt $EXP ] && { echo "  sleeping $((EXP-now+5))s past expiry"; sleep $((EXP-now+5)); }
RA=$(ROOT=$ROOT FEE_PAYER=$ROOT $TXGEN $ACC3_PK $CHAIN $(non $ACC3) $SINK 3 2>/dev/null | tail -1)
HA=$(cast rpc eth_sendRawTransaction "$RA" --rpc-url $RPC 2>&1 | tr -d '"' | head -c 90)
echo "  after-expiry AA tx submit -> $HA"
echo "  after-expiry receipt -> $(wait_rcpt $(echo "$HA" | grep -oE '0x[0-9a-f]{64}') 6) (expect none)"

echo "===== DONE (head=$(cast block-number --rpc-url $RPC)) ====="
