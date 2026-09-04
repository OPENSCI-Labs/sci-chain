#!/usr/bin/env bash
# Gracefully stop the SCI-on-Sepolia stack (L1 geth/Nimbus + L2 reth/op-node +
# batcher + Blockscout) before an EC2 stop/start — e.g. to toggle Nitro Enclaves
# (EnclaveOptions). Run this ON the box (ubuntu@54.255.70.252), NOT locally.
#
# Why ordered + long timeouts: every container is `restart=unless-stopped` with
# StopTimeout unset (docker default 10s). geth's state DB can need more than 10s
# to flush; a SIGKILL mid-flush forces a slow recovery/rewind next boot. We stop
# producers/consumers first, then the stateful DBs with generous SIGTERM windows.
# `docker stop -t N` = SIGTERM, wait up to N seconds, then SIGKILL — a clean exit
# returns well before N, so a large N is free insurance.
#
# Data is safe across EC2 stop/start: this instance is EBS-only (single root
# volume), no instance-store. Stopping does not delete anything.
#
# After this completes, the chain is HALTED. Next steps (run from a host with AWS
# creds, NOT here) — confirm 54.255.70.252 is an Elastic IP first:
#   aws ec2 stop-instances  --instance-ids i-066cbe9064d4cbfb5 --region ap-southeast-1
#   aws ec2 wait instance-stopped --instance-ids i-066cbe9064d4cbfb5 --region ap-southeast-1
#   aws ec2 modify-instance-attribute --instance-id i-066cbe9064d4cbfb5 \
#       --region ap-southeast-1 --enclave-options 'Enabled=true'
#   aws ec2 start-instances --instance-ids i-066cbe9064d4cbfb5 --region ap-southeast-1
# Then bring the stack back with safe-start.sh.
#
# Usage: ./safe-stop.sh [-y]    (-y / --yes skips the confirmation prompt)
set -uo pipefail

ASSUME_YES=0
[[ "${1:-}" == "-y" || "${1:-}" == "--yes" ]] && ASSUME_YES=1

# Stop a container only if it exists; otherwise note and skip. Never abort the
# whole sequence because one container is already gone.
dstop() {
  local timeout="$1"; shift
  for c in "$@"; do
    if docker inspect "$c" >/dev/null 2>&1; then
      printf '  stop -t %-3s %s\n' "$timeout" "$c"
      docker stop -t "$timeout" "$c" >/dev/null 2>&1 \
        || echo "    WARN: failed to stop $c (continuing)"
    else
      echo "  skip (absent)  $c"
    fi
  done
}

echo "=== SCI-on-Sepolia: graceful stop ==="
echo "This HALTS the chain (single sequencer). Data persists on EBS."
if [[ "$ASSUME_YES" -ne 1 ]]; then
  read -r -p "Proceed? [y/N] " ans
  [[ "$ans" =~ ^[Yy]$ ]] || { echo "Aborted."; exit 1; }
fi

echo
echo "-- Phase 1: producers / consumers (stateless or re-derivable) --"
# Batcher first so it is not mid L2->L1 submission.
dstop 30 base-batcher
# op-nodes before their ELs, so no engine-API call hits a closing reth.
dstop 60 base-builder-cl base-client-cl
# Blockscout app layer before its DB, so postgres has no live clients to wait on.
dstop 30 bs-nginx bs-frontend bs-web bs-rpc-shim bs-verifier l1-proxy

echo
echo "-- Phase 2: stateful DBs (data-critical, generous flush windows) --"
dstop 120 bs-db                       # postgres: clients gone -> fast clean checkpoint
dstop 180 base-builder base-client    # L2 reth (MDBX) flush
dstop 60  sepolia-nimbus              # L1 CL
dstop 300 sepolia-geth                # L1 EL: stop last, longest window

echo
echo "-- Remaining state of stack containers --"
docker ps -a --format '{{.Names}}\t{{.Status}}' \
  | grep -iE 'sepolia-(geth|nimbus)|base-|bs-|l1-proxy' | sort

echo
echo "All stack containers stopped. Safe to EC2 stop/start now."
echo "Bring back up afterwards with: ./safe-start.sh"
