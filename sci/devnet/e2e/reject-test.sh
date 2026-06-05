#!/usr/bin/env bash
# Single-variable regression: does ONE keychain-hook-rejected AA tx stall the chain?
#
# Submits one AA tx the keychain hook MUST reject (root=ACC2, signer=ACC4, no authorized
# key) and watches whether block production stalls. Before the fix (commit 25c485a92) a
# hook-rejected AA tx returned a Custom error the builder treated as fatal -> every
# flashblock build aborted -> chain wedged. After the fix the rejection is an
# InvalidTransaction the builder SKIPS, so the chain keeps producing and the tx stays
# pending/unmined. This guards against regressing that classification.
#
# Spec/context: sci/docs/plan-a-aa-e2e.md + the "hook-rejected AA tx wedges sequencer"
# root-cause notes. Expected POST-fix: head keeps advancing, tx never mined.
#
# Config (env overrides):
#   L2_RPC      devnet L2 RPC            (default http://localhost:8545; :7545 = sequencer)
#   L2_NODE_RPC rollup node (op-node)    (default http://localhost:8549; :7549 = sequencer node)
#   SCI_REPO    repo root                (default: inferred)
#   AA_TXGEN    sci-aa-txgen path        (default: $SCI_REPO/target/release/sci-aa-txgen)
#   CHAIN_ID    L2 chain id              (default 42001)
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

RPC=${L2_RPC:-http://localhost:8545}
SEQ=${L2_NODE_RPC:-http://localhost:8549}
SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)}
TXGEN=${AA_TXGEN:-$SCI_REPO/target/release/sci-aa-txgen}
CHAIN=${CHAIN_ID:-42001}

KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
ACC2=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
ACC4=0x15d34AAf54267DB7D7c367839AAf71A00a2C6A65 ; ACC4PK=0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a
SINK=0x6666666666666666666666666666666666666666

ss(){ curl -s $SEQ -X POST -H content-type:application/json -d '{"jsonrpc":"2.0","id":1,"method":"optimism_syncStatus","params":[]}' 2>/dev/null | python3 -c 'import sys,json;d=json.load(sys.stdin)["result"];print("unsafe="+str(d["unsafe_l2"]["number"])+" safe="+str(d["safe_l2"]["number"]))' 2>/dev/null; }
rc(){ cast rpc eth_getTransactionReceipt "$1" --rpc-url $RPC 2>/dev/null | python3 -c 'import sys,json;d=json.load(sys.stdin);print("MINED block "+str(int(d["blockNumber"],16))+" status="+d["status"]) if d else print("pending")' 2>/dev/null; }

echo "########## REJECTED-AA-TX STALL TEST (rpc=$RPC) ##########"
echo "=== BASELINE (no tx): chain producing? ==="
for i in $(seq 1 4); do echo "  head=$(cast block-number --rpc-url $RPC) $(ss)"; sleep 8; done

echo "=== submit ONE hook-rejected AA tx: root=ACC2, signer=ACC4, NO key authorized (unauthorized root) ==="
echo "  getKey ROOT->ACC4 (should be EMPTY): $(cast call $KEYCHAIN 'getKey(address,address)(uint8,address,uint64,bool,bool)' $ACC2 $ACC4 --rpc-url $RPC 2>&1 | tr '\n' ' ')"
N=$(cast nonce $ACC4 --rpc-url $RPC)
RAW=$(ROOT=$ACC2 FEE_PAYER=$ACC2 $TXGEN $ACC4PK $CHAIN $N $SINK 1 2>/dev/null | tail -1)
H=$(cast rpc eth_sendRawTransaction "$RAW" --rpc-url $RPC 2>&1 | tr -d '"')
echo "  submitted: $H"

echo "=== watch ~150s: head should KEEP advancing + tx stays unmined (post-fix) ==="
for i in $(seq 1 18); do
  echo "  iter=$i head=$(cast block-number --rpc-url $RPC) $(ss) | tx=$(rc $H)"
  sleep 8
done
echo "########## DONE ##########"
