#!/usr/bin/env bash
# Bridge Optimism-mainnet ETH -> native Sepolia ETH via the LayerZero testnetbridge
# SwappableBridge (SETH variant). Self-verifying; never prints the private key.
#
# Usage:
#   bridge-op-to-sepolia.sh --account BATCHER [--amount-eth 0.001] [--send]
#   bridge-op-to-sepolia.sh --to 0x.. --key 0x.. [--amount-eth 0.001] [--send]
#
# Defaults to a DRY RUN (cast call simulation only). Add --send to broadcast.
# Account mode reads <NAME>_ADDR / <NAME>_KEY from --env (default sci/sepolia/deploy/.env).
set -euo pipefail

OP_RPC="${OP_RPC:-https://mainnet.optimism.io}"
SEPOLIA_RPC="${SEPOLIA_RPC:-https://ethereum-sepolia-rpc.publicnode.com}"
# Verified OP->Sepolia bridge. oft() must equal SETH_OFT below; the sibling
# 0x0A9f824C05A74F577A536A8A0c673183a872Dff4 is the dead Goerli bridge — DO NOT use it.
BRIDGE=0x8352c746839699b1fc631fddc0c3a00d4ac71a17
SETH_OFT=0xE71bDfE1Df69284f00EE185cf0d95d0c7680c0d4
DST=161                          # Sepolia LayerZero v1 chain id
GAS_RESERVE=60000000000000       # 0.00006 ETH kept on OP for gas
ZERO=0x0000000000000000000000000000000000000000

ENV_FILE="sci/sepolia/deploy/.env"
ACCOUNT="" ; TO="" ; PK="" ; AMOUNT_ETH="" ; SEND=0

while [ $# -gt 0 ]; do
  case "$1" in
    --account)     ACCOUNT="$2"; shift 2;;
    --to)          TO="$2"; shift 2;;
    --key)         PK="$2"; shift 2;;
    --amount-eth)  AMOUNT_ETH="$2"; shift 2;;
    --env)         ENV_FILE="$2"; shift 2;;
    --op-rpc)      OP_RPC="$2"; shift 2;;
    --sepolia-rpc) SEPOLIA_RPC="$2"; shift 2;;
    --send)        SEND=1; shift;;
    *) echo "unknown arg: $1" >&2; exit 2;;
  esac
done

if [ -n "$ACCOUNT" ]; then
  [ -f "$ENV_FILE" ] || { echo "env file not found: $ENV_FILE" >&2; exit 1; }
  TO=$(grep -E "^${ACCOUNT}_ADDR=" "$ENV_FILE" | cut -d= -f2- | tr -d '"'"'"' \t\r\n')
  PK=$(grep -E "^${ACCOUNT}_KEY="  "$ENV_FILE" | cut -d= -f2- | tr -d '"'"'"' \t\r\n')
  [ -n "$TO" ] && [ -n "$PK" ] || { echo "could not read ${ACCOUNT}_ADDR/${ACCOUNT}_KEY from $ENV_FILE" >&2; exit 1; }
fi
[ -n "$TO" ] || { echo "need --account NAME or --to ADDR" >&2; exit 2; }

echo "== config =="
echo "to        : $TO"
echo "bridge    : $BRIDGE"
echo "op rpc    : $OP_RPC"
echo "mode      : $([ "$SEND" = 1 ] && echo SEND || echo 'DRY RUN (simulate only; add --send to broadcast)')"

# --- safety: confirm bridge is the SETH/Sepolia one, not Goerli ---
oft=$(cast call "$BRIDGE" 'oft()(address)' --rpc-url "$OP_RPC")
[ "${oft,,}" = "${SETH_OFT,,}" ] || { echo "ABORT: bridge.oft()=$oft != SETH_OFT $SETH_OFT" >&2; exit 1; }
remote=$(cast call "$SETH_OFT" 'trustedRemoteLookup(uint16)(bytes)' "$DST" --rpc-url "$OP_RPC")
[ "$remote" != "0x" ] && [ -n "$remote" ] || { echo "ABORT: SETH OFT has no trusted remote for dstChainId $DST" >&2; exit 1; }
echo "verified  : bridge bound to SETH OFT, Sepolia(161) trusted remote present ✓"

# --- amounts ---
BAL=$(cast balance "$TO" --rpc-url "$OP_RPC")
FEE=$(cast call "$SETH_OFT" 'estimateSendFee(uint16,bytes,uint256,bool,bytes)(uint256,uint256)' \
        "$DST" "$TO" 1000000000000000 false 0x --rpc-url "$OP_RPC" | head -1 | awk '{print $1}')
AMT_IN_WEI=""
[ -n "$AMOUNT_ETH" ] && AMT_IN_WEI=$(cast to-wei "$AMOUNT_ETH" ether)
read AMT VAL <<EOF
$(python3 - <<PY
bal=$BAL; fee=$FEE; reserve=$GAS_RESERVE
inc=fee*5//4
fixed="$AMT_IN_WEI"
amt = int(fixed) if fixed else bal - inc - reserve
val = amt + inc
if amt <= 0: raise SystemExit("ABORT: balance too low to cover fee+gas")
if val + 1 > bal: raise SystemExit("ABORT: value %d exceeds balance %d" % (val, bal))
print(amt, val)
PY
)
EOF
python3 - <<PY
bal=$BAL; fee=$FEE; amt=$AMT; val=$VAL
print("== amounts ==")
print("balance   : %d (%.6f ETH)"%(bal, bal/1e18))
print("nativeFee : %d (%.8f ETH), +25%% buffer"%(fee, fee/1e18))
print("amountIn  : %d (%.6f ETH)"%(amt, amt/1e18))
print("msg.value : %d (%.6f ETH)"%(val, val/1e18))
print("gas left  : %d (%.6f ETH)"%(bal-val, (bal-val)/1e18))
PY

# --- simulate (free) ---
echo "== simulate (cast call) =="
cast call --from "$TO" --value "$VAL" "$BRIDGE" \
  'swapAndBridge(uint256,uint256,uint16,address,address,address,bytes)' \
  "$AMT" 0 "$DST" "$TO" "$TO" "$ZERO" 0x --rpc-url "$OP_RPC" >/dev/null
echo "simulation OK (no revert) ✓"

if [ "$SEND" != 1 ]; then
  echo "DRY RUN complete. Re-run with --send to broadcast."
  exit 0
fi

[ -n "$PK" ] || { echo "ABORT: --send requires a key (--account or --key)" >&2; exit 1; }
DERIVED=$(cast wallet address --private-key "$PK")
[ "${DERIVED,,}" = "${TO,,}" ] || { echo "ABORT: key derives $DERIVED != $TO" >&2; exit 1; }

BASE=$(cast balance "$TO" --rpc-url "$SEPOLIA_RPC")
echo "== broadcasting on OP mainnet =="
TX=$(cast send "$BRIDGE" \
  'swapAndBridge(uint256,uint256,uint16,address,address,address,bytes)' \
  "$AMT" 0 "$DST" "$TO" "$TO" "$ZERO" 0x \
  --value "$VAL" --rpc-url "$OP_RPC" --private-key "$PK" --json \
  | python3 -c "import sys,json;d=json.load(sys.stdin);print(d['transactionHash'],d['status'])")
echo "op tx: $TX"

echo "== polling Sepolia delivery (baseline $BASE) =="
for i in $(seq 1 40); do
  B=$(cast balance "$TO" --rpc-url "$SEPOLIA_RPC" 2>/dev/null || true)
  if [ -n "$B" ] && [ "$B" != "$BASE" ]; then
    python3 -c "b=$B;base=$BASE;print('ARRIVED ~%ds: balance %.6f ETH (+%.6f)'%($i*15,b/1e18,(b-base)/1e18))"
    exit 0
  fi
  sleep 15
done
echo "no Sepolia change after ~10min — LZ relayer delayed; check balance later"
