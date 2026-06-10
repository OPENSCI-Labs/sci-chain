#!/usr/bin/env bash
# Fresh-machine clean deploy of the SCI devnet (single sequencer).
#
# Unlike redeploy.sh (which assumes an already-running stack where the L2 bootnodes
# and their ENR files already exist), this script brings the FULL stack up from
# scratch in dependency order — crucially the L2 **bootnodes**, which generate the
# `cl-bootnode.enr` that base-{builder,client}-cl require. On a brand-new machine
# redeploy.sh leaves the CL nodes crash-looping with
#   Error: Failed to read bootnodes file /bootnodes/cl-bootnode.enr
#
# TWO HARD RULES baked in (learned 2026-06-10 on the GPU box):
#  1. Start the bootnodes (base-el-bootnode + base-cl-bootnode) before the CL nodes.
#  2. EVERY `up` uses `--no-deps`. Bringing up ANY service WITHOUT --no-deps re-triggers
#     setup-l2 (a completed dependency); compose re-runs it, which REGENERATES
#     genesis.json and WIPES the SCI allocs applied in stage C. The EL db (alloc'd
#     genesis) then mismatches the chainspec (no-alloc genesis) and the EL crash-loops:
#       genesis hash in storage does not match the specified chainspec
#
# Run on the devnet host. Needs docker + the SCI images (base-{reth-node,builder}:sci,
# base-{consensus,batcher}:local, devnet-setup:local) + cast (foundry) + jq + python3.
# Set L2_CHAIN_ID=42001 in etc/docker/devnet-env BEFORE running (setup-l2 bakes it).
#
# Config (env overrides): SCI_REPO, L2_RPC (default :8545), L2_NODE_RPC (default :7549).
set -uo pipefail
export PATH=$PATH:~/.foundry/bin

SCI_REPO=${SCI_REPO:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}
cd "$SCI_REPO"
RPC=${L2_RPC:-http://localhost:8545}
SEQ_RPC=${L2_NODE_RPC:-http://localhost:7549}
C="docker compose --env-file etc/docker/devnet-env -f etc/docker/docker-compose.yml -f sci/devnet/docker-compose.sci.yml"
ts(){ date +%H:%M:%S; }
wait_health(){ # $1=container  $2=max 2s-iters (default 40)
  for _ in $(seq 1 "${2:-40}"); do
    [ "$(docker inspect "$1" --format '{{.State.Health.Status}}' 2>/dev/null)" = healthy ] && return 0
    sleep 2
  done; return 1; }

echo "[$(ts)] === A: down + wipe + L1 up ==="
$C down --remove-orphans >/dev/null 2>&1
sudo docker run --rm -v "$PWD/.devnet:/devnet" alpine sh -c 'rm -rf /devnet/* /devnet/.??*'
$C up -d l1-el l1-cl l1-vc >/dev/null 2>&1
wait_health l1-cl 40
echo "[$(ts)] l1-cl=$(docker inspect l1-cl --format '{{.State.Health.Status}}' 2>/dev/null)"

echo "[$(ts)] === B: setup-l2 (generate genesis) ==="
$C up -d setup-l2 >/dev/null 2>&1
for _ in $(seq 1 40); do docker inspect setup-l2 --format '{{.State.Status}}' 2>/dev/null | grep -q exited && break; sleep 3; done
echo "[$(ts)] setup-l2=$(docker inspect setup-l2 --format '{{.State.Status}} rc={{.State.ExitCode}}' 2>/dev/null)"

echo "[$(ts)] === C: apply SCI allocs (keychain precompile + 3 predeploys) ==="
sudo bash sci/devnet/apply-sci-allocs.sh .devnet/l2/configs/genesis.json 2>&1 | tail -1
sudo bash sci/devnet/apply-predeploy-allocs.sh .devnet/l2/configs/genesis.json 2>&1 | tail -1

echo "[$(ts)] === D: bootnodes (--no-deps; generates cl-bootnode.enr the CL needs) ==="
$C up -d --no-deps base-el-bootnode base-cl-bootnode >/dev/null 2>&1
wait_health base-cl-bootnode 60
echo "[$(ts)] cl-bootnode=$(docker inspect base-cl-bootnode --format '{{.State.Health.Status}}' 2>/dev/null)"

echo "[$(ts)] === E: EL up (--no-deps) + read genesis hash ==="
$C up -d --no-deps base-client base-builder >/dev/null 2>&1
for _ in $(seq 1 40); do cast block 0 --rpc-url "$RPC" >/dev/null 2>&1 && break; sleep 2; done
HASH=$(cast block 0 --rpc-url "$RPC" | awk '/^hash/{print $2}')
echo "[$(ts)] genesis hash=$HASH"

echo "[$(ts)] === F: patch rollup genesis l2 hash ==="
for f in rollup.json rollup-conductor.json; do
  P=.devnet/l2/configs/$f; [ -f "$P" ] || continue
  sudo jq --arg h "$HASH" '.genesis.l2.hash=$h' "$P" > /tmp/$f.tmp && sudo cp /tmp/$f.tmp "$P"
done

echo "[$(ts)] === G: CL + batcher up (--no-deps) ==="
$C up -d --no-deps base-client-cl base-builder-cl base-batcher >/dev/null 2>&1
echo "[$(ts)] L2 genesis age = $(( $(date +%s) - $(cast block 0 --rpc-url "$RPC" | awk '/^timestamp/{print $2}') ))s"

echo "[$(ts)] === verify head advances + safe tracks unsafe (NO CL restarts) ==="
ss(){ curl -s "$SEQ_RPC" -X POST -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"optimism_syncStatus","params":[]}' 2>/dev/null \
  | python3 -c "import sys,json;d=json.load(sys.stdin).get('result',{});print(d.get('unsafe_l2',{}).get('number'),d.get('safe_l2',{}).get('number'))" 2>/dev/null; }
for i in $(seq 1 18); do
  read -r u s <<< "$(ss)"
  echo "[$(ts)] iter=$i head=$(cast block-number --rpc-url "$RPC" 2>/dev/null) unsafe=${u:-?} safe=${s:-?}"
  sleep 10
done
echo "[$(ts)] DONE"
