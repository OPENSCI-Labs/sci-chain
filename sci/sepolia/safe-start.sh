#!/usr/bin/env bash
# Bring the SCI-on-Sepolia stack back up after an EC2 stop/start, in dependency
# order (L1 -> L2 EL -> op-node -> batcher -> Blockscout), with health gating.
# Run this ON the box (ubuntu@54.255.70.252).
#
# Why `docker start` and NOT `docker compose up`: these containers were explicitly
# `docker stop`-ed by safe-stop.sh, so `restart=unless-stopped` will NOT auto-start
# them on instance boot — we start them by name here. `docker compose up` would
# re-trigger the one-shot setup-l1/setup-l2 services and could disturb the genesis
# / deployment. `docker start` only resumes existing containers.
#
# Pairs with safe-stop.sh.
set -uo pipefail

# Host-mapped RPC ports (verified on the box).
# NOTE: geth's own HTTP RPC is :8645 — health-gate on THAT. :18645 is the socat
# proxy (l1-proxy container) that the in-docker op-nodes reach via
# host.docker.internal:18645; it is not up until l1-proxy starts, so gating on it
# would deadlock (the proxy starts in this very script).
L1_RPC=http://localhost:8645         # Sepolia geth (direct)
L2_BUILDER_RPC=http://localhost:7545 # L2 sequencer EL
L2_CLIENT_RPC=http://localhost:8545  # L2 verifier EL

dstart() {
  for c in "$@"; do
    if docker inspect "$c" >/dev/null 2>&1; then
      printf '  start %s\n' "$c"
      docker start "$c" >/dev/null 2>&1 || echo "    WARN: failed to start $c (continuing)"
    else
      echo "  skip (absent)  $c"
    fi
  done
}

# Poll an EL JSON-RPC endpoint until eth_chainId responds, or give up after ~N tries.
wait_rpc() {
  local name="$1" url="$2" tries="${3:-60}" i=0
  echo -n "  waiting for $name ($url) "
  while (( i < tries )); do
    if curl -s -m 5 -X POST -H 'Content-Type: application/json' \
         --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' "$url" \
         2>/dev/null | grep -q '"result"'; then
      echo "OK"; return 0
    fi
    echo -n "."; sleep 5; (( i++ ))
  done
  echo " TIMEOUT (continuing anyway)"; return 1
}

echo "=== SCI-on-Sepolia: ordered bring-up ==="

# Sanity: if the point of the reboot was to enable Nitro Enclaves, the device
# should now exist. Informational only — does not block the chain.
if [[ -e /dev/nitro_enclaves ]]; then
  echo "Nitro Enclaves: /dev/nitro_enclaves present."
else
  echo "Nitro Enclaves: /dev/nitro_enclaves NOT present (enclave not enabled / allocator not up)."
fi

echo
echo "-- L1 (geth + Nimbus) + l1-proxy --"
# l1-proxy (socat 18645->8645) must be up BEFORE the op-nodes, which reach L1 via
# host.docker.internal:18645 — start it here with L1, not in the Blockscout group.
dstart sepolia-geth sepolia-nimbus l1-proxy
wait_rpc "L1 geth" "$L1_RPC" 120   # L1 may need a moment to reopen its DB

echo
echo "-- L2 execution layer (reth) --"
dstart base-builder base-client
wait_rpc "L2 builder" "$L2_BUILDER_RPC" 60
wait_rpc "L2 client"  "$L2_CLIENT_RPC"  60

echo
echo "-- L2 consensus (op-node) + batcher --"
# op-nodes re-derive from L1 and catch up to head on their own.
dstart base-builder-cl base-client-cl
dstart base-batcher

echo
echo "-- Blockscout --"
dstart bs-db bs-verifier bs-web bs-rpc-shim bs-frontend bs-nginx

echo
echo "=== Health check ==="
docker ps --format '{{.Names}}\t{{.Status}}' \
  | grep -iE 'sepolia-(geth|nimbus)|base-|bs-|l1-proxy' | sort

l1=$(curl -s -m 5 -X POST -H 'Content-Type: application/json' \
  --data '{"jsonrpc":"2.0","method":"eth_chainId","params":[],"id":1}' "$L1_RPC" 2>/dev/null \
  | python3 -c 'import sys,json; print(int(json.load(sys.stdin)["result"],16))' 2>/dev/null || echo '?')
echo "L1 chainId = $l1 (expect 11155111)"

# L2 head + how fresh the latest block is — a stale timestamp means the sequencer
# is wedged (see sci/docs/key-lessons l1-reorg checklist), not just slow to start.
for pair in "builder:$L2_BUILDER_RPC" "client:$L2_CLIENT_RPC"; do
  name="${pair%%:*}"; url="${pair#*:}"
  out=$(curl -s -m 5 -X POST -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","method":"eth_getBlockByNumber","params":["latest",false],"id":1}' "$url" 2>/dev/null \
    | python3 -c '
import sys, json, time
b = json.load(sys.stdin)["result"]
n  = int(b["number"], 16)
ts = int(b["timestamp"], 16)
age = int(time.time()) - ts
print("block=%d ts_age=%ds %s" % (n, age, "FRESH" if age < 120 else "STALE?"))' 2>/dev/null || echo 'unreachable')
  echo "L2 $name: $out"
done

echo
echo "Done. If an L2 head shows STALE, restart the op-nodes:"
echo "  docker restart base-builder-cl base-client-cl"
