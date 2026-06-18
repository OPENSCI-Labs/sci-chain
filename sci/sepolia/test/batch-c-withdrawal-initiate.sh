#!/usr/bin/env bash
# Batch C1 — withdrawal INITIATION (TEST_PLAN F5.5, L2 side only).
# Initiates an L2->L1 withdrawal of the bridged-ETH ERC20 via L2StandardBridge.withdrawTo,
# and verifies: token burned on L2, L2ToL1MessagePasser emits MessagePassed (withdrawal
# queued), and sentMessages[withdrawalHash] == true. Produces the withdrawal hash.
#
# NOTE: prove + finalize on L1 are GATED on this deployment — no proposer runs
# (gameCount=0, no L1 output root), and even with one the window is ~7 days
# (proofMaturityDelaySeconds=604800). C1 verifies only the immediately-testable L2 side.
# Run ON the deploy host.
set -uo pipefail
export PATH=$PATH:~/.foundry/bin
SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." 2>/dev/null && pwd)}

L2=${L2:-http://localhost:8545}
L2BRIDGE=0x4200000000000000000000000000000000000010
L2MP=0x4200000000000000000000000000000000000016
L2ETH=${L2ETH:-0x79fca56b224f878a8b4119ecfc42c3d908ffdbbf}   # bridged ETH (paired to Sepolia WETH)
DEPLOYER=0xd339ffBf98D9f56Fb391f9130986DC5B8a2c282e          # holds the L2 ETH from the deposit test
DEV0=0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266               # SCI gas faucet
DEV0_KEY=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
DKEY=$(grep -E '^DEPLOYER_KEY=' "$SCI_REPO/sci/sepolia/deploy/.env" 2>/dev/null | cut -d= -f2- | tr -d '[:space:]')
AMT=5000000000000000   # 0.005 L2 ETH to withdraw

PASS=0; FAIL=0
ok(){ echo "  PASS: $1"; PASS=$((PASS+1)); }
no(){ echo "  FAIL: $1"; FAIL=$((FAIL+1)); }
bal_eth(){ cast call $L2ETH 'balanceOf(address)(uint256)' "$1" --rpc-url $L2 2>/dev/null | awk '{print $1}'; }

echo "== Batch C1: withdrawal initiation (L2 side) =="
[ -n "$DKEY" ] || { echo "  ABORT: cannot read DEPLOYER_KEY"; exit 1; }

echo "== preconditions =="
EB=$(bal_eth $DEPLOYER); echo "  deployer L2-ETH balance = $EB"
python3 -c "exit(0 if int('$EB') >= $AMT else 1)" 2>/dev/null && ok "deployer holds >= 0.005 L2 ETH" || { no "insufficient L2 ETH to withdraw"; echo "RESULT PASS=$PASS FAIL=$FAIL"; exit 1; }

# ensure deployer has SCI for gas (fund from dev0 if low)
SCIBAL=$(cast balance $DEPLOYER --rpc-url $L2 2>/dev/null)
if python3 -c "exit(0 if int('$SCIBAL') < 10000000000000000 else 1)" 2>/dev/null; then
  echo "  funding deployer 0.05 SCI for gas (from dev0)"
  cast send $DEPLOYER --value 50000000000000000 --rpc-url $L2 --private-key $DEV0_KEY --json >/dev/null 2>&1; sleep 3
fi

echo "== initiate withdrawal: L2StandardBridge.withdrawTo(L2_ETH, deployer, 0.005) =="
RC=$(cast send $L2BRIDGE 'withdrawTo(address,address,uint256,uint32,bytes)' $L2ETH $DEPLOYER $AMT 200000 0x \
  --rpc-url $L2 --private-key $DKEY --json 2>&1)
TXH=$(echo "$RC" | python3 -c 'import sys,json;print(json.load(sys.stdin)["transactionHash"])' 2>/dev/null)
ST=$(echo "$RC" | python3 -c 'import sys,json;print(json.load(sys.stdin).get("status",""))' 2>/dev/null)
echo "  tx=$TXH status=$ST"
[ "$ST" = "0x1" ] && ok "withdrawTo included (status 1)" || no "withdrawTo failed ($ST)"

echo "== verify burn + MessagePassed (withdrawal queued) =="
sleep 3
EB2=$(bal_eth $DEPLOYER)
python3 -c "exit(0 if int('$EB')-int('$EB2')==$AMT else 1)" 2>/dev/null && ok "L2-ETH burned 0.005 on withdraw ($EB -> $EB2)" || no "burn delta wrong ($EB -> $EB2)"
# pull the MessagePassed log from L2ToL1MessagePasser and extract withdrawalHash (4th word of data)
WHASH=$(cast rpc eth_getTransactionReceipt "$TXH" --rpc-url $L2 2>/dev/null | python3 -c "
import sys,json
r=json.load(sys.stdin)
mp='$L2MP'.lower()
for l in r['logs']:
    if l['address'].lower()==mp:
        d=l['data'][2:]
        # MessagePassed(nonce idx,sender idx,target idx, value, gasLimit, bytes data, bytes32 withdrawalHash)
        # non-indexed head: value, gasLimit, offset(data), withdrawalHash -> 4th 32-byte word
        print('0x'+d[3*64:4*64]); break
" 2>/dev/null)
echo "  withdrawalHash = ${WHASH:-<none>}"
[ -n "$WHASH" ] && [ "${WHASH:0:2}" = "0x" ] && [ "$WHASH" != "0x$(printf '0%.0s' {1..64})" ] && ok "L2ToL1MessagePasser emitted MessagePassed (withdrawal hash produced)" || no "no MessagePassed/withdrawalHash"
if [ -n "$WHASH" ]; then
  SENT=$(cast call $L2MP 'sentMessages(bytes32)(bool)' "$WHASH" --rpc-url $L2 2>/dev/null)
  [ "$SENT" = "true" ] && ok "sentMessages[withdrawalHash]=true (queued for L1 proof)" || no "withdrawal not queued (sentMessages=$SENT)"
fi

echo "== NOTE: prove/finalize is gated =="
echo "  - no proposer running / gameCount=0 -> no L1 output root to prove against yet"
echo "  - finalization window ~7 days (proofMaturityDelaySeconds=604800) once a proposer posts"
echo "  C1 verifies the L2-side initiation only (immediately testable)."

echo "== RESULT =="; echo "PASS=$PASS FAIL=$FAIL"
[ "$FAIL" -eq 0 ] && { echo "BATCH C1 GREEN ✓ (L2 withdrawal initiation verified; L1 finalize gated)"; exit 0; } || { echo "BATCH C1 RED ✗"; exit 1; }
