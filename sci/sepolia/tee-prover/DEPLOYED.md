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
