#!/usr/bin/env bash
# Clean wipe-genesis redeploy of the SCI devnet. Runs stages A–E back-to-back to keep the
# L2 genesis time close to wall-clock (L2 genesis time ≈ L1 genesis time = stage A), which
# avoids the sequencer building a huge catch-up backlog and stalling in
# `AwaitingSafeHeadConfirmation`.
#
# IMPORTANT ops lesson: do NOT restart the CL nodes (`base-{builder,client}-cl`) to "unstick"
# a drift stall — each restart reorgs the unconfirmed unsafe chain, which resets the batcher
# pipeline and makes the lag worse. If the chain drift-stalls, prefer a fresh back-to-back
# redeploy (this script). See the `project_devnet_v1_7_1_deployment` memory.
#
# Run on the devnet host (needs docker + the built `:sci` images for client/builder/CL).
# Config (env overrides):
#   SCI_REPO     repo root            (default: inferred from this script's location)
#   L2_RPC       L2 EL RPC            (default http://localhost:8545)
#   L2_NODE_RPC  sequencer rollup node (default http://localhost:7549; needs the node's
#                                       RPC exposed — e.g. a compose override)
#   EXTRA_COMPOSE  extra `-f <file>` compose args (e.g. a local debug override), optional
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
cd "$SCI_REPO"
RPC=${L2_RPC:-http://localhost:8545}
SEQ_RPC=${L2_NODE_RPC:-http://localhost:7549}
C="docker compose --env-file etc/docker/devnet-env -f etc/docker/docker-compose.yml -f sci/devnet/docker-compose.sci.yml ${EXTRA_COMPOSE:-}"
ts(){ date +%H:%M:%S; }

echo "[$(ts)] === A: down + wipe + L1 up ==="
$C down --remove-orphans >/dev/null 2>&1
sudo docker run --rm -v "$PWD/.devnet:/devnet" alpine sh -c 'rm -rf /devnet/* /devnet/.??*'
$C up -d l1-el l1-cl l1-vc >/dev/null 2>&1
echo "[$(ts)] waiting l1-cl healthy"
for i in $(seq 1 40); do [ "$(docker inspect l1-cl --format '{{.State.Health.Status}}' 2>/dev/null)" = healthy ] && break; sleep 3; done
echo "[$(ts)] l1-cl=$(docker inspect l1-cl --format '{{.State.Health.Status}}' 2>/dev/null)"

echo "[$(ts)] === B: setup-l2 ==="
$C up -d setup-l2 >/dev/null 2>&1
for i in $(seq 1 40); do docker inspect setup-l2 --format '{{.State.Status}}' 2>/dev/null | grep -q exited && break; sleep 3; done
echo "[$(ts)] setup-l2=$(docker inspect setup-l2 --format '{{.State.Status}} rc={{.State.ExitCode}}' 2>/dev/null)"

echo "[$(ts)] === C: allocs (precompiles + 3 predeploys) ==="
sudo bash sci/devnet/apply-sci-allocs.sh .devnet/l2/configs/genesis.json 2>&1 | tail -1
sudo bash sci/devnet/apply-predeploy-allocs.sh .devnet/l2/configs/genesis.json 2>&1 | tail -1

echo "[$(ts)] === D: EL up + genesis hash ==="
$C up -d --no-deps base-client base-builder >/dev/null 2>&1
for i in $(seq 1 30); do cast block 0 --rpc-url $RPC >/dev/null 2>&1 && break; sleep 2; done
HASH=$(cast block 0 --rpc-url $RPC | grep -iE '^hash' | awk '{print $2}')
echo "[$(ts)] genesis hash=$HASH"

echo "[$(ts)] === E: patch rollup genesis + CL + batcher ==="
for f in rollup.json rollup-conductor.json; do
  P=.devnet/l2/configs/$f
  sudo jq --arg h "$HASH" '.genesis.l2.hash=$h' "$P" > /tmp/$f.tmp && sudo cp /tmp/$f.tmp "$P"
done
$C up -d --no-deps base-client-cl base-builder-cl base-batcher >/dev/null 2>&1
echo "[$(ts)] all services up; L2 genesis age = $(( $(date +%s) - $(cast block 0 --rpc-url $RPC | grep -iE '^timestamp' | awk '{print $2}') ))s"

echo "[$(ts)] === verify safe tracks unsafe (NO CL restarts) ==="
ss(){ curl -s $SEQ_RPC -X POST -H 'content-type: application/json' -d '{"jsonrpc":"2.0","id":1,"method":"optimism_syncStatus","params":[]}' 2>/dev/null | python3 -c "import sys,json;d=json.load(sys.stdin).get('result',{});print(d.get('unsafe_l2',{}).get('number'),d.get('safe_l2',{}).get('number'))" 2>/dev/null; }
for i in $(seq 1 24); do
  read u s <<< "$(ss)"
  echo "[$(ts)] iter=$i head=$(cast block-number --rpc-url $RPC 2>/dev/null) unsafe=$u safe=$s gap=$(( ${u:-0} - ${s:-0} ))"
  sleep 10
done
echo "[$(ts)] DONE"
