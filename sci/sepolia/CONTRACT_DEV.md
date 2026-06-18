# Contract Development on SCI Chain (Sepolia testnet deployment)

SCI Chain is an Agent-native Ethereum L2 (Base Azul v0.9 fork, OP-Stack) with **Sepolia
as its L1**. This document lists everything you need to develop and deploy contracts on it.

> Status: live testnet. Chain may be reset; treat all state as disposable.

## 1. Network connection

| Field | Value |
|---|---|
| Chain name | SCI Chain (Sepolia testnet) |
| **Chain ID** | **42001** (`0xA411`) |
| L2 RPC (HTTP) | `http://54.255.70.252:8545` |
| L2 RPC (WS) | `ws://54.255.70.252:8546` |
| Rollup node (op-node) RPC | `http://54.255.70.252:8549` (`optimism_syncStatus`, etc.) |
| Block explorer | `http://54.255.70.252:4000` (Blockscout) |
| Block time | ~2 s |
| Block gas limit | 60,000,000 |
| Min base fee | 1 gwei |
| **Native gas token** | **SCI** (NOT ETH) — see §3 |
| L1 | Ethereum **Sepolia** (chain ID `11155111`) |

> Access note: the RPC ports are bound on the host; reachability depends on the cloud
> security group. If `:8545` is not reachable directly, tunnel it:
> `ssh -L 8545:localhost:8545 ubuntu@54.255.70.252` and use `http://localhost:8545`.
> `:8545` is the validator EL (read + send tx). `:7545` is the sequencer EL (same chain).

Add to MetaMask / wallet: Network name `SCI Chain`, RPC `http://54.255.70.252:8545`,
Chain ID `42001`, currency symbol `SCI`, explorer `http://54.255.70.252:4000`.

## 2. Dev / test accounts (pre-funded)

The standard Foundry/Hardhat mnemonic accounts are funded with **10,000 SCI** each on L2
genesis (`fundDevAccounts = true`). Use them for deployment and testing.

Mnemonic: `test test test test test test test test test test test junk`

| # | Address | Private key |
|---|---|---|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |

> These are public well-known keys — for testnet dev only, never on a value-bearing chain.
> To fund a fresh account, send SCI from a funded dev account (it is the native gas coin).

## 3. Native gas token — SCI (Custom Gas Token)

Gas is paid in **SCI**, not ETH (OP-Stack Custom Gas Token v2). Practical implications:

- `eth_getBalance` / `address.balance` / `msg.value` are denominated in **SCI** (18 decimals).
- Wallets should label the currency `SCI`.
- Native ETH bridging from L1 is disabled; the canonical asset is SCI. CGT liquidity is
  managed by the predeploys below (`NativeAssetLiquidity` / `LiquidityController`).
- Everything else is standard EVM — `payable`, `transfer`, `call{value:}` all work, moving SCI.

## 4. Contract addresses on SCI Chain (L2)

### SCI Agent-permission system (the differentiator)

| Address | Contract | Type |
|---|---|---|
| `0xAAAAAAAA00000000000000000000000000000000` | AccountKeychain | Rust precompile |
| `0xAAAAAAAA00000000000000000000000000000001` | SciAgentState (CircuitBreaker trip state) | Rust precompile |
| `0xBBBBBBBB00000000000000000000000000000001` | AgentAccessKeyRegistry | Solidity predeploy |
| `0xBBBBBBBB00000000000000000000000000000002` | AgentBudgetController | Solidity predeploy |
| `0xBBBBBBBB00000000000000000000000000000003` | AgentCircuitBreaker | Solidity predeploy |

Public interfaces live in `sci/contracts/src/interfaces/` (e.g. `IAccountKeychain.sol`).
Query the keychain directly, e.g.:
```bash
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)(uint8,uint64,bool,bool)' <root> <sessionKey> \
  --rpc-url http://54.255.70.252:8545
```

### Custom Gas Token (CGT v2) predeploys

| Address | Contract |
|---|---|
| `0x4200000000000000000000000000000000000029` | NativeAssetLiquidity (premined SCI liquidity) |
| `0x420000000000000000000000000000000000002A` | LiquidityController (owner-controlled) |

### Standard OP-Stack L2 predeploys

All the usual `0x420000000000000000000000000000000000000X` predeploys are present
(`L2CrossDomainMessenger` `..07`, `L2StandardBridge` `..10`, `L1Block` `..15`,
`GasPriceOracle` `..0F`, `WETH` `..06`, `OptimismMintableERC20Factory` `..12`, etc.).

## 5. L1 (Sepolia) contracts

For L1↔L2 messaging / bridging tooling:

| Contract (L1 Sepolia) | Address |
|---|---|
| OptimismPortalProxy | `0xd4b05f9944dd530965e0a7cd66af205e13b69036` |
| L1StandardBridgeProxy | `0xd515748082854aa6e0bd468d4075150a6e87f5aa` |
| L1CrossDomainMessengerProxy | `0xe2f0001c93adfa14412fbcc9c0a796cdc748dcd4` |
| SystemConfigProxy | `0xea2ffcaa6370cf35aff530fc79871c0beaf95aa9` |
| DisputeGameFactoryProxy | `0x69a8e8137d8f5a35ba0670192738816c3031ec52` |

Full set: `sci/sepolia/deploy/<workdir>/l1-addresses.json` on the deploy host.

## 6. Deploying contracts

Standard Solidity / EVM — Foundry or Hardhat both work. Gas is just paid in SCI.

```bash
export L2_RPC=http://54.255.70.252:8545          # or http://localhost:8545 via tunnel
export PK=0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80  # dev acct 0

# deploy
forge create --rpc-url $L2_RPC --private-key $PK src/MyContract.sol:MyContract

# or scripted
forge script script/Deploy.s.sol --rpc-url $L2_RPC --private-key $PK --broadcast

# call / send
cast call <addr> 'totalSupply()(uint256)' --rpc-url $L2_RPC
cast send <addr> 'mint(address,uint256)' <to> 1000 --rpc-url $L2_RPC --private-key $PK
```

The SCI Foundry project lives in `sci/contracts/` (`forge build`, `forge test`).

## 7. Fork / EVM compatibility

- Base Azul (OP-Stack) feature set; **Base Azul / Osaka fork is active** (activated at L2
  block 20, so live for all current blocks). Standard Solidity ≥0.8.x compiles and runs.
- EVM semantics are stock except for the SCI precompiles and the SCI native gas token.
- Solidity contracts do not need to know about agents/keychain unless they want to
  integrate with the agent-permission system.

## 8. Agent-native features (optional, SCI-specific)

If you are building for AI-agent access (the reason SCI exists):

- **Agent transactions ride a native AA tx type `0x76`** (`BaseAaTransaction`): a batch of
  `calls[]` plus an optional `fee_payer` (sponsored gas). A Rust pre-execution hook applies
  per-call keychain checks (CircuitBreaker → Scope → SpendingLimit) before execution; on any
  violation the batch fails fast (only intrinsic gas spent).
- The session key model: a root account authorizes session keys (via `AccountKeychain` /
  `AgentAccessKeyRegistry`) with scopes (allowed targets/selectors) and spending limits
  (per-token quotas, including a native-SCI sentinel for gas + value).
- Recipient restrictions apply to any `transfer`/`approve`-shaped call on any token target
  (SCI treats every such target as token-like).
- A JS/TS encoder for `0x76` txs exists in the SCI SDK tooling; standard EOAs use ordinary
  EIP-1559 txs as usual.

Your ordinary contracts need none of this. It only matters when an agent (session key) is
the caller and you want protocol-enforced permissioning.

## 9. Caveats

- **Testnet, resettable** — do not rely on persistence; redeploys may wipe L2 state.
- **Sepolia blobs prune after ~18 days** — historical data availability for old ranges
  needs an archive; batches are currently posted as L1 **calldata**, not blobs.
- The safe head lags the unsafe head by a derivation/confirmation window (tens of L2
  blocks); use the unsafe head (`eth_blockNumber`) for fast UX, the safe head
  (`optimism_syncStatus`) when you need L1-backed finality.
- No fault-proof / proposer service runs in this permissioned deployment.
