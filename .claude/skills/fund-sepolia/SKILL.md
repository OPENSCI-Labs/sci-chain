---
name: fund-sepolia
description: Fund a Sepolia testnet account by bridging Optimism-mainnet ETH to native Sepolia ETH via the LayerZero testnetbridge SwappableBridge, using cast (no web frontend, no MetaMask). Use when the user wants Sepolia test ETH or to top up sci-chain-on-Sepolia role accounts (batcher/proposer/challenger/sequencer/deployer). Source funds are real OP-mainnet ETH on the target address.
---

# fund-sepolia

Bridge **real Optimism-mainnet ETH → native Sepolia ETH** for a testnet account, entirely
from the CLI. This exists because the testnetbridge.com web UI frequently fails with
`failed to fetch`, and MetaMask can't be driven here. The yield is large: ~0.001 OP ETH
historically delivered ~520 Sepolia ETH (the SETH/WETH pool prices the testnet OFT cheaply).

Role-account keys live in `sci/sepolia/deploy/.env` (gitignored): `BATCHER_ADDR`/`BATCHER_KEY`,
`PROPOSER_*`, `CHALLENGER_*`, `SEQUENCER_*`, `DEPLOYER_*`. Related: see the
`ref-op-to-sepolia-testnetbridge` memory and `project_sepolia_node_experiment`.

## Prerequisites
- `cast` (foundry) and `python3` on PATH.
- The target address already holds OP-mainnet ETH (ask the user to send some first;
  `cast balance <addr> --rpc-url https://mainnet.optimism.io` to confirm).

## How to run

Always dry-run first (simulates via `cast call`, spends nothing), then `--send`.

```bash
# dry run for the batcher account (reads BATCHER_ADDR/BATCHER_KEY from the .env)
.claude/skills/fund-sepolia/scripts/bridge-op-to-sepolia.sh --account BATCHER

# broadcast: swap (almost) the whole OP balance, poll until Sepolia delivery
.claude/skills/fund-sepolia/scripts/bridge-op-to-sepolia.sh --account BATCHER --send

# swap a fixed amount instead of the full balance
.claude/skills/fund-sepolia/scripts/bridge-op-to-sepolia.sh --account PROPOSER --amount-eth 0.001 --send

# ad-hoc address + key (no .env)
.claude/skills/fund-sepolia/scripts/bridge-op-to-sepolia.sh --to 0x.. --key 0x.. --send
```

Flags: `--account NAME` | `--to ADDR --key 0x..` · `--amount-eth X` (default = balance − fee − gas
reserve) · `--env PATH` (default `sci/sepolia/deploy/.env`) · `--op-rpc` / `--sepolia-rpc` · `--send`.

## What the script guarantees
- **Uses the correct bridge.** `0x8352c746839699b1fc631fddc0c3a00d4ac71a17` (oft = SETH
  "Sepolia ETH"). It asserts `bridge.oft() == SETH_OFT` and that the OFT has a non-empty
  `trustedRemoteLookup(161)` before doing anything. The look-alike sibling
  `0x0A9f824C05A74F577A536A8A0c673183a872Dff4` is bound to the **dead Goerli OFT** and would
  burn gas for nothing — the script refuses it.
- **Simulates before sending** (`cast call`); aborts on any revert.
- **Verifies the key** derives the target address before broadcasting; never prints the key.
- `amountOutMin = 0`, `adapterParams = 0x`, `dstChainId = 161` — matches the official
  LayerZero hardhat task (testnet OFT, no MEV risk).
- Polls the Sepolia balance (~15s–5min) and reports the delivered amount.

## Operator notes
- Delivery is via the LayerZero relayer; usually arrives in well under a minute.
- If `failed`/revert appears in simulation, re-check the target has enough OP ETH to cover
  `amountIn + LZ fee + gas`.
- Defaults keep `0.00006` ETH on OP for gas. Bump `GAS_RESERVE` in the script if OP L1 data
  fees spike.
