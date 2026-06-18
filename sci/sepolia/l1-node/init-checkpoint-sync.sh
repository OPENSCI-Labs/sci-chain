#!/usr/bin/env bash
# One-shot Nimbus checkpoint sync (trustedNodeSync) — run ONCE before the
# first `docker compose up -d`. --backfill=false: a fresh L2 only needs L1
# history forward from deploy time, keeping the CL DB at tens of GB.
set -euo pipefail

DATA_DIR=/home/ubuntu/sepolia/nimbus-data
CHECKPOINT_URL=${CHECKPOINT_URL:-https://sepolia.checkpoint-sync.ethpandaops.io}

mkdir -p "$DATA_DIR"
chmod 700 "$DATA_DIR"

docker run --rm \
  -v "$DATA_DIR":/data \
  statusim/nimbus-eth2:multiarch-latest \
  trustedNodeSync \
  --network=sepolia \
  --data-dir=/data \
  --trusted-node-url="$CHECKPOINT_URL" \
  --backfill=false
