# CLAUDE.md — SCI Chain Development Guide

## Project

SCI Chain is an Agent-native Ethereum L2, forked from Base Azul v0.8 (`base/base`).
It adds a protocol-level permission sandbox for AI Agents via the Keychain Precompile
(ported from Tempo v1.6.0), with MPP (Machine Payments Protocol) as the Agent access layer.

Chain ID: 42001 | Rust edition: 2024 | Rust version: 1.93.1 | Linker: mold

## Architecture

```
Agent → mppx.fetch() → SCI Agent Gateway (MPP 402 + REST)
                              ↓ JSON-RPC
                        SCI Chain (Base Azul v0.8 fork)
                          Pre-execution hook: CircuitBreaker → Scope → SpendingLimit
                          Precompile: 0xAAAA.. AccountKeychain
                          Predeploys: 0xBBBB01 Registry, 0xBBBB02 Budget, 0xBBBB03 Breaker
                          0xCCCC01 SCIAgentDelegator (EIP-7702)
                          IDA contract (ERC-721 + ERC-6551 TBA)
```

## Repository Structure

```
sci-chain/
├── crates/                    ← Base original Rust code (DO NOT add files here)
│   └── execution/evm/src/lib.rs  ← ONLY Base file we modify (precompile registration)
├── etc/docker/devnet-env      ← Modified: Chain ID 42001
├── sci/                       ← ALL SCI additions go here
│   ├── crates/                ←   Rust (Keychain precompile)
│   │   ├── precompiles/       ←     Core: AccountKeychain, storage abstraction
│   │   ├── precompiles-macros/←     Proc macros (#[contract], #[Storable])
│   │   └── contracts/         ←     ABI bindings (alloy sol!)
│   ├── contracts/             ←   Solidity (Foundry project)
│   │   ├── src/agent/         ←     P0-2: AccessKeyRegistry, BudgetController, CircuitBreaker
│   │   ├── src/integration/   ←     P0-4: SciAgentRegistrar, SCIAgentDelegator
│   │   └── src/interfaces/    ←     Public interfaces (other repos depend on these)
│   ├── gateway/               ←   TypeScript (MPP Server + REST API)
│   ├── devnet/                ←   Genesis patch + allocs
│   └── docs/                  ←   Project documentation
└── Cargo.toml                 ← Modified: workspace members include sci/crates/*
```

## Critical Rules

1. **Never add files to Base directories** (crates/, bin/, devnet/, etc/, docs/, actions/, baseup/).
   All SCI code goes under `sci/`.
2. **Only 3 Base files are modified**:
   - `Cargo.toml` — workspace members
   - `crates/execution/evm/src/lib.rs` — precompile registration
   - `etc/docker/devnet-env` — Chain ID
3. **Tempo code is reference only**. Source is at `~/sci-dev/Tempo-ref/`.
   Copy and adapt, never import as dependency.
4. **Namespace convention**: all Tempo references must be renamed:
   - `tempo_precompiles` → `sci_precompiles`
   - `tempo_precompiles_macros` → `sci_precompiles_macros`
   - `tempo_contracts` → `sci_contracts`
   - `TempoHardfork` → remove or replace with feature flag
   - `TIP-20` → standard ERC-20

## Build Commands

```bash
# Rust — check SCI crates only (fast)
cargo check -p sci-precompiles -p sci-precompiles-macros -p sci-contracts

# Rust — check entire workspace (slow, includes Base)
cargo check

# Rust — build release binary
cargo build --release --bin based

# Rust — run SCI tests
cargo nextest run -p sci-precompiles

# Solidity — build contracts
cd sci/contracts && forge build

# Solidity — run tests
cd sci/contracts && forge test -vvv

# Gateway — dev server
cd sci/gateway && npm run dev

# Devnet — start (requires Docker)
just devnet up-single

# Devnet — status
just devnet status
```

## Precompile Addresses

| Address | Contract | Type |
|---|---|---|
| `0xAAAAAAAA00000000000000000000000000000000` | AccountKeychain | Precompile (Rust) |
| `0xBBBBBBBB00000000000000000000000000000001` | AgentAccessKeyRegistry | Predeploy (Solidity) |
| `0xBBBBBBBB00000000000000000000000000000002` | AgentBudgetController | Predeploy (Solidity) |
| `0xBBBBBBBB00000000000000000000000000000003` | AgentCircuitBreaker | Predeploy (Solidity) |
| `0xCCCCCCCC00000000000000000000000000000001` | SCIAgentDelegator | Predeploy (Solidity, EIP-7702) |

## Key Rust Files (SCI)

- `sci/crates/precompiles/src/lib.rs` — Precompile registration entry point
- `sci/crates/precompiles/src/account_keychain/mod.rs` — Core keychain logic (4331 lines, from Tempo)
- `sci/crates/precompiles/src/account_keychain/dispatch.rs` — ABI selector routing (373 lines)
- `sci/crates/precompiles/src/storage/` — EVM storage abstraction (8651 lines, from Tempo)
- `sci/crates/precompiles-macros/src/lib.rs` — Proc macros (3326 lines, from Tempo)

## Key Solidity Files (SCI)

- `sci/contracts/src/agent/AgentAccessKeyRegistry.sol` — keyId ↔ agentId binding
- `sci/contracts/src/agent/AgentBudgetController.sol` — budget query + alerts
- `sci/contracts/src/agent/AgentCircuitBreaker.sol` — trip/reset emergency freeze
- `sci/contracts/src/integration/SciAgentRegistrar.sol` — ERC-8004 one-step registration
- `sci/contracts/src/integration/SCIAgentDelegator.sol` — EIP-7702 batch executor
- `sci/contracts/src/interfaces/IAccountKeychain.sol` — Precompile interface

## Key Base File We Modify

`crates/execution/evm/src/lib.rs`:
- Find `PrecompilesMap` setup in `BaseEvmConfig`
- Add `sci-precompiles` to dependencies in `crates/execution/evm/Cargo.toml`
- Register `0xAAAA...` → `AccountKeychain::execute` in the precompile map
- Add pre-execution hook for scope/limit/breaker checks

## Commit Convention

| Prefix | Meaning |
|---|---|
| `sci:` | New SCI feature |
| `sci-fix:` | SCI bug fix |
| `base-mod:` | Modify Base original code |
| `contracts:` | Solidity contract changes |
| `gateway:` | MPP/REST gateway changes |
| `test:` | Tests |
| `devnet:` | Devnet configuration |

## Branches

- `main` — stable, protected (PR + 1 review required)
- `feat/p0-1-keychain` — R's Keychain precompile work
- `feat/p0-2-contracts` — S's Solidity contract work
- `feat/p0-3-gateway` — S's MPP Gateway work

## Test Accounts (Devnet)

Mnemonic: `test test test test test test test test test test test junk`

| # | Address | Private Key |
|---|---|---|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |

## Reth Version Note

- Base v0.8 uses `reth @ v1.11.4` (tagged release)
- Tempo v1.6.0 uses `reth @ dbb8495` (nightly rev)
- Both require Rust 1.93.1, edition 2024
- Trait signatures may differ — check compatibility before assuming copy-paste works

## Common Tasks

### Copy Keychain code from Tempo
```bash
TEMPO=~/sci-dev/Tempo-ref
SCI=~/sci-dev/sci-chain

# Storage
cp -r $TEMPO/crates/precompiles/src/storage/* $SCI/sci/crates/precompiles/src/storage/

# Keychain
cp $TEMPO/crates/precompiles/src/account_keychain/mod.rs $SCI/sci/crates/precompiles/src/account_keychain/
cp $TEMPO/crates/precompiles/src/account_keychain/dispatch.rs $SCI/sci/crates/precompiles/src/account_keychain/

# Macros
cp $TEMPO/crates/precompiles-macros/src/*.rs $SCI/sci/crates/precompiles-macros/src/

# Error types
cp $TEMPO/crates/precompiles/src/error.rs $SCI/sci/crates/precompiles/src/

# Then rename all: tempo_ → sci_
find sci/crates -name "*.rs" -exec sed -i 's/tempo_precompiles/sci_precompiles/g' {} +
find sci/crates -name "*.rs" -exec sed -i 's/tempo_contracts/sci_contracts/g' {} +
```

### Deploy contract to devnet
```bash
cd sci/contracts
export L2_RPC=http://localhost:8545
export PRIV0=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80
forge create --rpc-url $L2_RPC --private-key $PRIV0 src/agent/AgentAccessKeyRegistry.sol:AgentAccessKeyRegistry
```

### Call keychain precompile on devnet
```bash
# Query a key (view function)
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)(uint8,uint64,bool,bool)' \
  0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 \
  0x70997970C51812dc3A010C7d01b50e0d17dc79C8 \
  --rpc-url http://localhost:8545
```

## What NOT to do

- Do not run `cargo fmt` on Base original files (creates massive diffs)
- Do not update Base's `rust-toolchain.toml`
- Do not modify `crates/consensus/` or `crates/builder/` (no need for Plan B)
- Do not introduce a new transaction type (we use Plan B: standard tx + precompile)
- Do not import Tempo crates as git dependencies (copy + adapt instead)
