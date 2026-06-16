#!/usr/bin/env bash
# CGT v2 — ETH->L2 bridge e2e (reproduces the 2026-06-16 devnet verification).
#
# Proves the SAFE/CORRECT way to move "ETH" to an L2 whose native gas token is
# SCI (OP-Stack Custom Gas Token v2):
#   (neg) native ETH deposit via OptimismPortal  -> MUST revert (CGT mode)
#   (pos) ERC-20 standard bridge                  -> arrives on L2 as an
#         OptimismMintableERC20 (a token, NOT native; native stays SCI).
#
# Run ON the devnet host (needs docker + foundry). The devnet must be a CGT
# chain (deploy-fresh.sh with [chains.customGasToken] in the l2-intent).
#
# Verified result (54.255, 2026-06-16): neg reverts OptimismPortal_NotAllowedOnCGTMode
# (0xbd58e0a2); pos: L1 lock 5 -> L2 mint 5 to recipient. PASS.
set -uo pipefail
export PATH="$PATH:$HOME/.foundry/bin"

L1_RPC="${L1_RPC:-http://localhost:4545}"
L2_RPC="${L2_RPC:-http://localhost:8545}"
# devnet test-junk acct1 (funded on both L1 ETH and L2 SCI)
KEY="${KEY:-0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d}"
TO="${TO:-0x70997970C51812dc3A010C7d01b50e0d17dc79C8}"
AMOUNT="${AMOUNT:-5000000000000000000}"   # 5e18
FACTORY="0x4200000000000000000000000000000000000012"  # OptimismMintableERC20Factory
ADDR_FILE="${ADDR_FILE:-$HOME/sci-dev/sci-chain/.devnet/l2/configs/l1-addresses.json}"
SRC_DIR="$(cd "$(dirname "$0")" && pwd)"

jqaddr() { sudo cat "$ADDR_FILE" | python3 -c "import json,sys;d={k.lower():v for k,v in json.load(sys.stdin).items()};print(d['$1'.lower()])"; }
PORTAL="$(jqaddr OptimismPortalProxy)"
BRIDGE="$(jqaddr L1StandardBridgeProxy)"
SYSCFG="$(jqaddr SystemConfigProxy)"
echo "L1=$L1_RPC L2=$L2_RPC PORTAL=$PORTAL BRIDGE=$BRIDGE"

echo "== precheck: CGT active on L1 =="
cast call "$SYSCFG" 'isCustomGasToken()(bool)' --rpc-url "$L1_RPC"

echo "== (neg) native ETH deposit MUST revert =="
if cast call "$PORTAL" 'depositTransaction(address,uint256,uint64,bool,bytes)' "$TO" "$AMOUNT" 100000 false 0x --value "$AMOUNT" --from "$TO" --rpc-url "$L1_RPC" 2>&1 | grep -qi "NotAllowedOnCGTMode\|reverted"; then
  echo "  PASS: native ETH deposit blocked (CGT)"
else
  echo "  FAIL: native ETH deposit was NOT blocked"; exit 1
fi

echo "== 1) deploy test ERC-20 on L1 =="
tmp="$(mktemp -d)"; mkdir -p "$tmp/src"; printf '[profile.default]\nsrc="src"\nout="out"\n' >"$tmp/foundry.toml"
cp "$SRC_DIR/TestERC20.sol" "$tmp/src/"
L1TOK="$(cd "$tmp" && forge create src/TestERC20.sol:TestERC20 --rpc-url "$L1_RPC" --private-key "$KEY" --broadcast 2>&1 | awk '/Deployed to:/{print $3}')"
echo "  L1 token = $L1TOK"

echo "== 2) create paired OptimismMintableERC20 on L2 =="
L2TOK="$(cast call "$FACTORY" 'createOptimismMintableERC20(address,string,string)(address)' "$L1TOK" "Bridged ETH (test)" "bETH" --rpc-url "$L2_RPC")"
cast send "$FACTORY" 'createOptimismMintableERC20(address,string,string)(address)' "$L1TOK" "Bridged ETH (test)" "bETH" --private-key "$KEY" --rpc-url "$L2_RPC" >/dev/null
echo "  L2 token = $L2TOK  remoteToken=$(cast call "$L2TOK" 'remoteToken()(address)' --rpc-url "$L2_RPC")"

echo "== 3) approve + depositERC20To on L1 =="
cast send "$L1TOK" 'approve(address,uint256)' "$BRIDGE" "$AMOUNT" --private-key "$KEY" --rpc-url "$L1_RPC" >/dev/null
cast send "$BRIDGE" 'depositERC20To(address,address,address,uint256,uint32,bytes)' "$L1TOK" "$L2TOK" "$TO" "$AMOUNT" 200000 0x --private-key "$KEY" --rpc-url "$L1_RPC" >/dev/null
echo "  L1 bridge locked = $(cast call "$L1TOK" 'balanceOf(address)(uint256)' "$BRIDGE" --rpc-url "$L1_RPC")"

echo "== 4) poll L2 balance until deposit is derived/minted =="
for n in $(seq 1 30); do
  bal="$(cast call "$L2TOK" 'balanceOf(address)(uint256)' "$TO" --rpc-url "$L2_RPC" 2>/dev/null | awk '{print $1}')"
  echo "  [$n] L2 bETH = $bal"
  [ -n "$bal" ] && [ "$bal" != "0" ] && { echo "  PASS: $bal minted on L2 (ERC-20, not native)"; exit 0; }
  sleep 5
done
echo "  FAIL: bridged token did not arrive on L2"; exit 1
