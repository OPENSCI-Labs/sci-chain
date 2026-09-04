#!/usr/bin/env bash
# Produces a genesis-alloc JSON fragment containing the 3 SCI fixed-address Solidity
# predeploys, ready to be merged into the L2 genesis.json. Compose with the existing
# sci-allocs.json (which seeds the two precompile addresses 0xAAAA..00/01) via the
# apply-sci-allocs.sh helper.
#
# Usage from the repo root:
#   bash sci/devnet/export-predeploy-allocs.sh > sci/devnet/sci-predeploy-allocs.json
#
# Optional env vars:
#   CB_OWNER  Address baked into AgentCircuitBreaker.storage[0] (Ownable._owner).
#             Defaults to devnet test-account-0 (0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266).
#
# Why this approach:
# - `AgentCircuitBreaker` at 0xBBBB..03 is load-bearing: the Rust SciAgentState
#   precompile gates `tripKey/untripKey` on `msg.sender == 0xBBBB..03`, so it MUST
#   land at its fixed address.
# - The other two (Registry, BudgetController) are observers; baking them at their
#   spec'd addresses is for consistency.
# - For all three, the runtime bytecode (`deployedBytecode`) is self-contained — no
#   constructor immutables to thread. Only `AgentCircuitBreaker` has constructor
#   state (Ownable._owner at slot 0), which we seed in `storage`.

set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
REPO_ROOT=$(cd "$SCRIPT_DIR/../.." && pwd)
CONTRACTS_DIR="$REPO_ROOT/sci/contracts"

CB_OWNER=${CB_OWNER:-0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266}

OWNER_LOWER=${CB_OWNER#0x}
OWNER_LOWER=${OWNER_LOWER,,}
if [[ ${#OWNER_LOWER} -ne 40 ]]; then
  echo "error: CB_OWNER must be a 0x-prefixed 20-byte address (got: $CB_OWNER)" >&2
  exit 2
fi
OWNER_SLOT_VALUE="0x000000000000000000000000${OWNER_LOWER}"

(cd "$CONTRACTS_DIR" && forge build >/dev/null 2>&1)

bytecode() {
  local name=$1
  (cd "$CONTRACTS_DIR" && forge inspect "$name" deployedBytecode)
}

REGISTRY_CODE=$(bytecode AgentAccessKeyRegistry)
BUDGET_CODE=$(bytecode AgentBudgetController)
BREAKER_CODE=$(bytecode AgentCircuitBreaker)

cat <<EOF
{
  "0xbbbbbbbb00000000000000000000000000000001": {
    "nonce": "0x1",
    "balance": "0x0",
    "code": "${REGISTRY_CODE}"
  },
  "0xbbbbbbbb00000000000000000000000000000002": {
    "nonce": "0x1",
    "balance": "0x0",
    "code": "${BUDGET_CODE}"
  },
  "0xbbbbbbbb00000000000000000000000000000003": {
    "nonce": "0x1",
    "balance": "0x0",
    "code": "${BREAKER_CODE}",
    "storage": {
      "0x0000000000000000000000000000000000000000000000000000000000000000": "${OWNER_SLOT_VALUE}"
    }
  }
}
EOF
