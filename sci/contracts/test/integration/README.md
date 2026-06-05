# Integration tests — SCI base contracts on devnet

This directory holds Foundry tests that target the **live SCI devnet** via
`forge --fork-url`. They run against the genesis-baked bytecode at the 3 fixed
predeploy addresses while mocking the Rust precompiles (which forking can't
reproduce). For the end-to-end agent loop that needs the real Rust pre-execution
hook, see `sci/devnet/e2e/` instead.

## What's covered

| File | Purpose |
|---|---|
| `Smoke.t.sol` | Chain ID, predeploy presence, CB owner, account funding |
| `DeploymentParity.t.sol` | Live runtime bytecode at `0xBBBB..01/02/03` byte-equals current `forge inspect deployedBytecode` |
| `CircuitBreaker.t.sol` | Trip/untrip flows, guardian model, access control + fuzz |
| `Registry.t.sol` | bindKey / unbindKey semantics + fuzz over (keyId, agentId) |
| `Budget.t.sol` | Threshold + alert behavior + fuzz over (threshold, remaining) |
| `Invariants.t.sol` | Cross-contract invariants (trip-state mirror, owner non-zero) |
| `Stress.t.sol` | Heavy-load: 100-entry bindings, 200 thresholds, 100 trip cycles |
| `base/DevnetBase.sol` | Shared base contract (addresses, accounts, mock-install helpers) |

For the end-to-end agent-tx loop that exercises the Rust hook under Plan A (native
AA tx `0x76`, no EIP-7702), see `sci/devnet/e2e/` and the runbook
`sci/docs/plan-a-aa-e2e.md`.

## Running

```bash
export L2_RPC=http://54.255.70.252:7545   # builder RPC of the SCI devnet
cd sci/contracts

# Everything under test/integration/
FOUNDRY_PROFILE=integration forge test --fork-url $L2_RPC -vv

# Or one file at a time
forge test --fork-url $L2_RPC --match-path 'test/integration/Smoke.t.sol' -vv

# Just fuzz tests (Foundry auto-picks any `testFuzz_*` function)
FOUNDRY_PROFILE=integration forge test --fork-url $L2_RPC --match-test 'testFuzz_' -vv

# Just invariants
FOUNDRY_PROFILE=integration forge test --fork-url $L2_RPC --match-contract InvariantsTest -vv

# Stress (lower fuzz budget; concrete bodies do the work)
FOUNDRY_PROFILE=stress forge test --fork-url $L2_RPC --match-path 'test/integration/Stress.t.sol' -vv
```

## How tests stay isolated from live state

Every test starts by `vm.etch`-ing fresh `MockAccountKeychain` and
`MockSciAgentState` contracts over the precompile addresses (see
`base/DevnetBase.sol::setUp`). The mocks have empty storage at the precompile
slots, so tests see a clean keychain regardless of how many `authorizeKey`
calls the live chain has accumulated.

`vm.makeAddrAndKey("<seed>")` is used to mint fresh session keys per test —
no two tests share a key, so binding/unbinding state can't leak across cases.

## When `forge test` (without --fork-url) is invoked

`DevnetBase.setUp` checks `block.chainid != SCI_CHAIN_ID` and calls
`vm.skip(true)` if there's no fork. So plain `forge test` will simply skip the
integration suite — `test/*.t.sol` unit tests run as usual.

## End-to-end agent loop (live devnet)

The integration tests here cover **Solidity behavior under the SCI predeploy
addresses**. They do NOT cover:

- The Rust pre-execution hook (CircuitBreaker check, Scope check, SpendingLimit
  pre-flight) — needs a real chain
- The native AA tx (`0x76`) carrier, signature recovery, and atomic batch execution

Those are exercised by the Plan A agent-loop e2e scripts in `sci/devnet/e2e/`
(`e2e-loop.sh` = register → AA transfer → limit → circuit breaker → expiry;
`reject-test.sh` = a hook-rejected AA tx must not wedge the chain). Run with:

```bash
L2_RPC=http://localhost:8545 sci/devnet/e2e/e2e-loop.sh
```

Those scripts mutate devnet state (key authorize, trip/untrip, etc.) — see the
runbook `sci/docs/plan-a-aa-e2e.md` for per-phase expected output, and the
`project_devnet_v1_7_1_deployment` memory for a fresh-genesis redeploy.
