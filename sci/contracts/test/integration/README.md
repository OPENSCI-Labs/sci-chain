# Integration tests — SCI base contracts on devnet

This directory holds Foundry tests that target the **live SCI devnet** via
`forge --fork-url`. They run against the genesis-baked bytecode at the 4 fixed
predeploy addresses while mocking the Rust precompiles (which forking can't
reproduce). For tests that need the real Rust pre-execution hook, see
`../../script/integration/` instead.

## What's covered

| File | Purpose |
|---|---|
| `Smoke.t.sol` | Chain ID, predeploy presence, CB owner, account funding |
| `DeploymentParity.t.sol` | Live runtime bytecode at `0xBBBB..01/02/03` and `0xCCCC..01` byte-equals current `forge inspect deployedBytecode` |
| `CircuitBreaker.t.sol` | Trip/untrip flows, guardian model, access control + fuzz |
| `Registry.t.sol` | bindKey / unbindKey semantics + fuzz over (keyId, agentId) |
| `Budget.t.sol` | Threshold + alert behavior + fuzz over (threshold, remaining) |
| `Delegator.t.sol` | Fail-closed direct call (MissingTransactionKey) + fuzz |
| `Invariants.t.sol` | Cross-contract invariants (trip-state mirror, owner non-zero) |
| `Stress.t.sol` | Heavy-load: 100-entry bindings, 200 thresholds, 100 trip cycles |
| `base/DevnetBase.sol` | Shared base contract (addresses, accounts, mock-install helpers) |

For the end-to-end agent-tx loop that exercises the Rust hook +
`SCIAgentDelegator` execute path, see
`sci/contracts/script/integration/AgentTxLoop.s.sol`.

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

## Live-broadcast counterparts

The integration tests here cover **Solidity behavior under the SCI predeploy
addresses**. They do NOT cover:

- The Rust pre-execution hook (CircuitBreaker check, Scope check, SpendingLimit
  pre-flight) — needs real chain
- EIP-7702 set-code tx + delegator dispatch — needs real chain
- Cross-frame `transaction_key` propagation between hook → delegator

Those live in `sci/contracts/script/integration/AgentTxLoop.s.sol`. Run with:

```bash
forge script script/integration/AgentTxLoop.s.sol \
  --rpc-url $L2_RPC --broadcast -vvv
```

That script will mutate devnet state (key authorize, 7702 install, etc.) — see
`sci/devnet/E2E.md` for the cleanup procedure or just run against a fresh
genesis.
