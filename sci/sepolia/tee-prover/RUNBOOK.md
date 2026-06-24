# SCI TEE Prover — Step 3: deploy multiproof contracts on Sepolia L1 (SHADOW mode)

Deploys the AggregateVerifier multiproof stack and registers it as **game type 621**
on the **already-live** OP-Stack DisputeGameFactory, in dev-bypass + shadow mode
(no AWS Nitro attestation; `respectedGameType` is NOT flipped, so the existing
permissioned withdrawal path is unaffected).

This runbook is run **on the deploy host** (`54.255.70.252`), where Sepolia geth is
reachable at `http://localhost:8645` and the deployer key is available. The artifacts
were prepared and compile-verified locally on 2026-06-24; see
`sci/docs/upgrade/sci-tee-prover-sepolia-deployment-plan.md` §0/§3.

## Locked values

| Item | Value |
|---|---|
| Deployer / DGF owner / SystemConfig guardian+owner | `0xd339ffBf98D9f56Fb391f9130986DC5B8a2c282e` (verified on-chain) |
| Live DisputeGameFactory | `0x69A8E8137D8F5a35Ba0670192738816C3031Ec52` |
| Live AnchorStateRegistry | `0x38eE07A983F73BC2ad116b6295E46A5ddC675695` |
| Game type | 621 |
| Init bond | 0.001 ETH |
| Block interval / intermediate | 600 / 30 |
| teeImageHash (= keccak256(PCR0)) | `0x10e7bc20a7249b37435cf4ac18e4d3aad5be8a04853f1d8dec4da3edbb43cf0d` |
| multiproofConfigHash (= 42001 config_hash) | `0x818e510a1bfc188acd00769740506fdc2ed3e8cbdb1a72c4b2c2acb0e4c2c6c7` |
| teeProposer | `0x3fA5F5DcC2474F0F77fA5916dc5bd4C46935971A` |
| teeChallenger | `0xFF6E90Ed75e1c3142c55Ef35687191a26DD1e6A5` |
| DelayedWETH | MockDelayedWETH (shadow; swap to real before flipping respectedGameType) |
| Cost | ~0.05–0.2 Sepolia ETH |

## Prerequisites (on host)

- `base-contracts` at `bdf7ab00`, foundry 1.7.1, lib submodules present (`just deps`).
  The RiscZero/NitroEnclave libs are NOT needed (dev bypass) — the build skips them.
- Sepolia geth at `http://localhost:8645` synced to head.
- `DEPLOYER_KEY` available (in `sci/sepolia/deploy/.env`).

## 1. Apply the bundle to base-contracts

From `sci/sepolia/tee-prover/bundle/`, copy into the host's `base-contracts`:

```bash
BC=/path/to/base-contracts            # the host's base-contracts checkout
cp deploy-config/sci-sepolia.json              "$BC/deploy-config/"
cp scripts-multiproof/DeploySciLive.s.sol      "$BC/scripts/multiproof/"
( cd "$BC" && git apply /path/to/bundle/DeployDevBase.virtual.patch )   # makes _deployInfrastructure virtual
```

## 2. Compile

```bash
cd "$BC"
forge build scripts/multiproof/DeploySciLive.s.sol --skip '*RiscZero*' '*NitroEnclave*'
# expect: "Compiler run successful!"
```

## 3. Dry-run (simulate — NO broadcast). REQUIRED before broadcast.

```bash
cd "$BC"
DEPLOY_CONFIG_PATH=deploy-config/sci-sepolia.json forge script \
  scripts/multiproof/DeploySciLive.s.sol:DeploySciLive \
  --rpc-url http://localhost:8645 \
  --sender 0xd339ffBf98D9f56Fb391f9130986DC5B8a2c282e \
  --skip '*RiscZero*' '*NitroEnclave*'
```

**What to check in the simulation:**
- It reaches `=== SCI LIVE DEPLOYMENT COMPLETE (NO NITRO) ===` without revert.
- `setImplementation(621, aggVerifier)` and `setInitBond(621, 0.001 ETH)` simulate OK
  (owner = sender, so no `onlyOwner` revert).
- **Anchor risk (the main reason to simulate):** the AggregateVerifier reads an anchor
  for game type 621 from the live ASR. Game type 621 is brand-new, so confirm the
  constructor + a simulated first-game creation do not revert on a missing anchor. If
  they do, set/seed the anchor for type 621 (or reconsider attaching to the live ASR
  vs. a controlled mock ASR) before broadcasting.

## 4. Broadcast (outward-facing — only after the dry-run is clean AND user confirms)

```bash
cd "$BC"
source /path/to/sci-chain/sci/sepolia/deploy/.env   # provides DEPLOYER_KEY
DEPLOY_CONFIG_PATH=deploy-config/sci-sepolia.json forge script \
  scripts/multiproof/DeploySciLive.s.sol:DeploySciLive \
  --rpc-url http://localhost:8645 \
  --private-key "$DEPLOYER_KEY" \
  --broadcast --skip '*RiscZero*' '*NitroEnclave*'
```

Deployed addresses print to console and to `deployments/11155111-sci-live.json`
(TEEProverRegistry, TEEVerifier, AggregateVerifier, DelayedWETH). The
`setImplementation` + `setInitBond` calls happen inside the broadcast.

## 5. Post-broadcast — register the dev signer (Step 6, after enclave is up)

Needs the `DevTEEProverRegistry` proxy address (from step 4 output) and the enclave
signer address (from Step 5: `enclave_signerPublicKey`).

```bash
cast send <DevTEEProverRegistry> \
  "addDevSigner(address,bytes32)" \
  <ENCLAVE_SIGNER_ADDRESS> \
  0x10e7bc20a7249b37435cf4ac18e4d3aad5be8a04853f1d8dec4da3edbb43cf0d \
  --rpc-url http://localhost:8645 --private-key "$DEPLOYER_KEY"
```

After this, `isValidSigner(<ENCLAVE_SIGNER_ADDRESS>)` returns true and the proposer
(Step 7) can submit type-621 games. Shadow mode: do NOT flip `respectedGameType` until
the type-621 games are observed resolving correctly.
