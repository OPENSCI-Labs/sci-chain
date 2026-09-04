#!/usr/bin/env bash
# Print (and optionally append to a log) the sync progress of the local
# Sepolia reth + Nimbus pair. Intended for a cron line like:
#   */30 * * * * /home/ubuntu/sepolia/sync-status.sh >> /home/ubuntu/sepolia/sync-progress.log 2>&1
#
# EL is reth 2.x (Storage V2, --minimal). reth uses staged sync, so during the
# Headers/Bodies stages eth_syncing reports all-zero block numbers; the reth log
# tail (last stage line) is the more useful progress signal until Execution catches up.
set -uo pipefail

EL=http://127.0.0.1:8645
CL=http://127.0.0.1:5152

ts=$(date -u +%Y-%m-%dT%H:%M:%SZ)

el_sync=$(curl -s -m 5 -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":1}' "$EL" \
  | python3 -c '
import sys, json
r = json.load(sys.stdin).get("result")
if r is False:
    print("SYNCED")
elif r is None:
    print("no-response")
elif isinstance(r, dict):
    cur = int(r.get("currentBlock","0x0"), 16)
    high = int(r.get("highestBlock","0x0"), 16)
    print("staged current=%d highest=%d" % (cur, high))
else:
    print("syncing")' 2>/dev/null || echo unreachable)

el_head=$(curl -s -m 5 -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_blockNumber","params":[],"id":1}' "$EL" \
  | python3 -c 'import sys,json; print(int(json.load(sys.stdin)["result"],16))' 2>/dev/null || echo '?')

el_peers=$(curl -s -m 5 -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}' "$EL" \
  | python3 -c 'import sys,json; print(int(json.load(sys.stdin)["result"],16))' 2>/dev/null || echo '?')

# reth staged-sync progress: last stage line from the container log
el_stage=$(docker logs sepolia-reth --since 2m 2>&1 | grep -oiE "sync::stages::[a-z]+|Received headers.*to_block=[0-9]+|Executing stage.*" | tail -1 || echo '-')

cl_sync=$(curl -s -m 5 "$CL/eth/v1/node/syncing" \
  | python3 -c '
import sys, json
d = json.load(sys.stdin)["data"]
print("head_slot=%s distance=%s syncing=%s el_offline=%s" % (d["head_slot"], d["sync_distance"], d["is_syncing"], d.get("el_offline","?")))' 2>/dev/null || echo unreachable)

# reth runs as root in its container, so its datadir needs sudo to measure
disk=$(sudo du -sh /home/ubuntu/sepolia/reth-data /home/ubuntu/sepolia/nimbus-data 2>/dev/null | awk '{printf "%s=%s ", $2, $1}')

echo "$ts | EL: $el_sync head=$el_head peers=$el_peers [$el_stage] | CL: $cl_sync | $disk"
