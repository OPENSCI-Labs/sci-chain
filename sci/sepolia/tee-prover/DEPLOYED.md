# SCI multiproof Step 3 — DEPLOYED on Sepolia L1 (2026-06-24)

Broadcast via `DeploySciLive.s.sol` (dev-bypass + shadow). `ONCHAIN EXECUTION COMPLETE
& SUCCESSFUL`, ~0.02 ETH. Run on `13.251.15.192` host geth via SSH tunnel; deployer
`0xd339ffBf98D9f56Fb391f9130986DC5B8a2c282e`.

## Deployed contracts (L1 = Sepolia, 11155111)

| Contract | Address |
|---|---|
| AggregateVerifier (game type 621 impl) | `0x78E74aF01024B351558C04A38de4Dc6Cf78FD11E` |
| DevTEEProverRegistry (proxy) | `0x4a5d38A941719e2133d3b77456bac99bce5e2997` |
| TEEVerifier | `0xc5078fC7c8f5ad52014F472E7F696f7f6cA3A4Cf` |
| MockDelayedWETH | `0xF7DF3103DF7d44c7beD4311FBDB42d5ea2cE3F38` |
| AnchorStateRegistry (live, reused) | `0x38eE07A983F73BC2ad116b6295E46A5ddC675695` |
| DisputeGameFactory (live, reused) | `0x69A8E8137D8F5a35Ba0670192738816C3031Ec52` |

## On-chain state after deploy (verified)

| Query | Value | Meaning |
|---|---|---|
| `DGF.gameImpls(621)` | `0x78E74aF0…` | AggregateVerifier registered as game type 621 |
| `DGF.initBonds(621)` | 0.001 ETH | init bond set |
| `OptimismPortal.respectedGameType()` | **1** (unchanged) | permissioned withdrawals still honored |
| `DGF.gameImpls(1)` | `0x58bf355C…` (unchanged) | permissioned impl intact |

Shadow mode: game type 621 is additive. The existing permissioned (type 1) withdrawal
path is unaffected; `respectedGameType` was NOT flipped.

## Key transactions

| Step | Tx hash |
|---|---|
| `setImplementation(621, AggregateVerifier)` | `0x1e58bc7c7bf25a6ff221bf20c60618ea1ffbf02a39d7dffdb58a787f4229309d` |
| `setInitBond(621, 0.001 ETH)` | `0x85a67a6eeff1bef79640acb304a7b5a75615cefe9c695ce12835099a25475b35` |
| AggregateVerifier CREATE | `0x694507faecefff843db084437cff093238fbec9bf6b339b207d30c83fe54d300` |
| DevTEEProverRegistry proxy CREATE | `0x26bac48326b73e6a3cca064eedf9ed1266dcf203d5c2335c8db4d9e869584c67` |

Full record: `11155111-sci-live.json` (this dir) +
`base-contracts/broadcast/DeploySciLive.s.sol/11155111/run-latest.json`.

## Step 5 — enclave + host UP (2026-06-24, host 13.251.15.192)

- Enclave RUNNING: `nitro-cli run-enclave --cpu-count 4 --memory 16384 --eif-path
  ~/sci-prover.eif --enclave-cid 16` (prod mode, PCR0 `4db46a64…`).
- nitro-host LISTENING `0.0.0.0:9555` (built locally, scp'd, `setsid nohup ... server`;
  env L1=:8645 L2=:8545 beacon=:5152 chain=42001 VSOCK_CID=16 registry=0x4a5d38A9…).
- vsock verified: `enclave_signerPublicKey` → 65B pubkey.
- **Enclave signer = `0x1585c1d8cdcc95a0ebb14256b9a6299ba9192e4e`** (ephemeral — regenerated
  on every enclave restart from NSM RNG; re-register after any restart).

## Step 6 — signer registered (2026-06-24, dev bypass)

`addDevSigner(0x1585c1d8…, 0x10e7bc20…)` by deployer (registry owner) →
tx `0x3d50b0d9f3649f5dfca05228731636e2420de1174f57ec710e1bcc3ab0766736`, status 1.
Verified: `isRegisteredSigner=true`, `signerImageHash=0x10e7bc20…`, host `/healthz` → HTTP 200.
Did NOT use the RISC0/Boundless `prover-registrar` path (dev bypass).

## Next
- **Step 7 — proposer** flags: `--anchor-state-registry-addr 0x38eE07…`,
  `--dispute-game-factory-addr 0x69A8E8…`, `--game-type 621`,
  `--tee-prover-registry-address 0x4a5d38A9…`,
  `--tee-image-hash 0x10e7bc20…`. The first type-621 game creation is where the live
  ASR anchor for the new game type gets exercised (the runtime risk to watch).

## Step 7 — minBaseFee fix + AggregateVerifier redeploy (2026-06-26)

The 6-24 stack above was superseded by a fresh chain + TEE redeploy on 6-25 (new
addresses below). Step 7 then surfaced two blockers, both now resolved.

### Blocker 1 — proposer `RLP error: input too short` (FIXED, root cause in code)

The prover guest reconstructed L2 block 1's `extraData` with `min_base_fee = 0`, but the
chain runs `min_base_fee = 1 gwei`. Only `extraData` diverged → guest block-1 hash ≠
canonical → block-2 derivation failed (`RLP input too short`). Fix:
`crates/common/chains/src/config.rs` now carries a `genesis_min_base_fee: Option<u64>`
field on `ChainConfig` (SCI = `Some(1_000_000_000)`, the 4 Base chains = `None`), folded
into `chain_genesis()`'s `SystemConfig`. EIF rebuilt → derivation now succeeds.
**`config_hash` is unaffected** — `PerChainConfig::marshal_binary` does not include
`min_base_fee`.

### Blocker 2 — `no valid signer found` (FIXED, required verifier redeploy)

`TEEProverRegistry.isValidSigner = isRegisteredSigner && signerImageHash == AggregateVerifier.TEE_IMAGE_HASH()`,
and `TEE_IMAGE_HASH` is **immutable**. Rebuilding the EIF rotated PCR0 → imageHash, so the
deployed AggregateVerifier (pinned to the old hash) had to be redeployed.
**Lesson: any EIF/genesis change rotates PCR0 → imageHash → the AggregateVerifier must be
redeployed and game type 621 re-pointed.**

### Live addresses (6-25 redeploy, host 13.251.15.192)

| Item | Value |
|---|---|
| DisputeGameFactory | `0x94fC1366051124abd364A4E32D6E11Bb23D1e95B` |
| AnchorStateRegistry | `0xd8a8b53F3C6B6AC51E977Fe5D67Ed1E9C346F45c` |
| DevTEEProverRegistry | `0xFeAd663fee9530ee1fe4934c59d238B09CdA7fe0` |
| TEEVerifier | `0x20CCc2c5e93E934A7864da4eD7340E89aA9704A7` |
| ZK Verifier | `0xb91C2E62d60372B4608A600a67230142A352BF0f` |
| MockDelayedWETH | `0xF2Ffe701884f90f38A5aBa9e1eC7B67F7B4f28D0` |
| AggregateVerifier (621, OLD, pinned to old image) | `0xD698d6c72e091775B0e72688DCC1593Ee2a7521E` |
| **AggregateVerifier (621, NEW, image 0x3100d466)** | **`0x169f35987128821F0F0dF188985A699e40Ecf754`** |
| CONFIG_HASH (multiproof) | `0x523fba43550eef7cdb12b8e55d537d6420f165e2b7a58687bb57a226e602a966` |

### EIF / enclave / signer (2026-06-26)

| Item | Value |
|---|---|
| EIF | `~/sci-prover-minbasefee.eif` (built from `~/sci-dev/sci-chain`) |
| PCR0 | `24ad6019080a3912ad7482337ea155eb9bf6cdf05daef6d04f85388fdaddc5de9ddb02ce5c94cc83106e965ef3fea0e2` |
| teeImageHash (= keccak256(PCR0)) | `0x3100d466c93472679b3106539b9d2e6e863ccefffe2d4a3cd3517b4ad12fc652` |
| Enclave signer (ephemeral) | `0x517b80bd823d3bf554847ab5eeca298481d16951` |

### Key transactions (2026-06-26)

| Step | Tx hash |
|---|---|
| `addDevSigner(0x517b80bd…, 0x3100d466…)` | `0x4316d01d6623a75e362555059150f77ec05edf616daa3992108d9bd65765adfb` |
| AggregateVerifier (new) CREATE | `0x1f6d62b84d38de9400c909442fc38867ccecdb0b6a400170b38bbb255bb06e7f` |
| `setImplementation(621, 0x169f3598…)` | `0xfa24ca901ae74e1cf3a7c318ce5ca0a527ab35f4adf6887431a0dd2d040bf2d7` |
| `setInitBond(621, 0.001 ETH)` | `0xf09150bacdea0477ea3104188d499d7bbc30d7df21d03d8091f6409de8668fdf` |

After the redeploy: `isValidSigner(0x517b80bd…) = true`, `DGF.gameImpls(621) = 0x169f3598…`.
Proposer restarted with `--tee-image-hash 0x3100d466…` and stopped rejecting the signer —
proving now proceeds (first type-621 game creation pending verification).
