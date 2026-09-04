#!/usr/bin/env bash
# Merges sci/devnet/sci-allocs.json into a generated L2 genesis.json.
#
# Run AFTER `setup-l2` has produced .devnet/l2/configs/genesis.json and BEFORE
# base-client / base-builder start (they read genesis.json on first boot).
#
# Usage from inside the running devnet stack (~/sci-dev/base-v0.8/):
#
#   bash ~/sci-dev/sci-chain/sci/devnet/apply-sci-allocs.sh \
#     .devnet/l2/configs/genesis.json
#
# What it does:
# - Backs up genesis.json to genesis.json.pre-sci
# - jq-merges every entry in sci-allocs.json into .alloc
# - Verifies the keychain (0xAAAA...0000) and SciAgentState (0xAAAA...0001)
#   addresses are present with code "0xef" (Tempo-compatible non-empty marker)
#
# Why "0xef" code:
# - SCI precompile state lives at addresses outside Ethereum's special-cased
#   precompile range (0x01-0x09 in revm), so EIP-161 garbage-collects "empty"
#   accounts at end-of-tx — silently dropping precompile sstore writes.
# - A 1-byte INVALID opcode (0xef) marks the account non-empty without making
#   it a callable contract: direct calls revert immediately on the invalid op,
#   precompile dispatch still wins because the address is in the precompile map.
# - Tempo dev.json uses the same pattern.

set -euo pipefail

GENESIS=${1:-}
if [[ -z "$GENESIS" ]]; then
  echo "usage: $0 <path-to-genesis.json>" >&2
  exit 2
fi

if [[ ! -f "$GENESIS" ]]; then
  echo "error: $GENESIS not found" >&2
  exit 2
fi

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ALLOCS="$SCRIPT_DIR/sci-allocs.json"
if [[ ! -f "$ALLOCS" ]]; then
  echo "error: $ALLOCS not found" >&2
  exit 2
fi

# Backup before mutation
cp -p "$GENESIS" "$GENESIS.pre-sci"

# Merge SCI allocs into .alloc. jq's '*' deep merge would also merge nested
# storage; we want SCI entries to fully replace any pre-existing entry, so use
# '+'. Since these addresses won't be in the upstream alloc, '+' is safe.
TMP=$(mktemp)
jq --slurpfile sci "$ALLOCS" '.alloc += $sci[0]' "$GENESIS" >"$TMP"
mv "$TMP" "$GENESIS"

# Verify
for addr in 0xaaaaaaaa00000000000000000000000000000000 0xaaaaaaaa00000000000000000000000000000001; do
  code=$(jq -r --arg a "$addr" '.alloc[$a].code // "MISSING"' "$GENESIS")
  if [[ "$code" != "0xef" ]]; then
    echo "error: alloc[$addr].code = $code (expected 0xef)" >&2
    exit 3
  fi
  echo "ok: alloc[$addr].code = $code"
done

echo "SCI allocs merged into $GENESIS (backup at $GENESIS.pre-sci)"
