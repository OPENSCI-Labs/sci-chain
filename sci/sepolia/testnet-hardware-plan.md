# SCI Chain Public Testnet — Node Hardware Plan

Target: public testnet launch (August 2026), settled on **Sepolia** L1.
Scope: hardware/topology only. Software stack per box = the existing devnet images
(`base-builder`, `base-client`, `base-consensus`, `base-batcher`) + SCI services.
Assumed region: ap-southeast-1 (same as current devnet server). Prices are
on-demand ballparks; a 1-year savings plan cuts ~35%.

## Components to host

| Component | Services | IO profile |
|---|---|---|
| L1 Sepolia node | geth (snap+path) + Nimbus (pruned) | ~400 GB, modest after initial sync |
| Sequencer | base-builder (EL) + base-builder-cl (rollup node) + base-batcher | latency-sensitive random reads; 2 s blocks / 250 ms flashblocks |
| RPC / verifier | base-client (EL) + base-client-cl + proxyd | read-heavy, scales horizontally |
| Services | SCI Agent Gateway (MPP/REST), Blockscout (+postgres, rpc-shim), faucet, Prometheus/Grafana | postgres is the main consumer |
| ZK prover | SP1 CUDA (Groth16/compressed) | **rent on demand** (L40S, e.g. g6e.4xlarge) — not a 24/7 box |

## Tier A — minimal (3 boxes, ~US$1.1k/mo) — acceptable for launch

| # | Role | Instance | RAM | Disk | ~$/mo |
|---|---|---|---|---|---|
| 1 | Sequencer (+batcher) | **i4i.2xlarge** | 64 GB | 1.75 TB **local NVMe** | ~560 |
| 2 | L1 node + public RPC | m7i.2xlarge | 32 GB | 1 TB gp3 (L1) + 500 GB gp3 (L2) | ~510 |
| 3 | Services (gateway, explorer, faucet, monitoring) | m7i.xlarge | 16 GB | 500 GB gp3 | ~230 |

Trade-off: public RPC shares a box with the L1 node — an RPC traffic spike can
degrade the L1 feed that the sequencer depends on. Fine for launch volume;
split per Tier B when RPC traffic grows.

## Tier B — recommended (5 boxes, ~US$2.1k/mo)

| # | Role | Instance | RAM | Disk | ~$/mo |
|---|---|---|---|---|---|
| 1 | Sequencer (+batcher) | **i4i.2xlarge** | 64 GB | 1.75 TB local NVMe | ~560 |
| 2 | L1 Sepolia node | r7i.xlarge | 32 GB | 1 TB gp3 | ~340 |
| 3 | RPC/verifier #1 | m7i.2xlarge | 32 GB | 500 GB gp3 | ~410 |
| 4 | RPC/verifier #2 | m7i.2xlarge | 32 GB | 500 GB gp3 | ~410 |
| 5 | Services | m7i.2xlarge | 32 GB | 500 GB gp3 | ~410 |

RPC #1/#2 sit behind proxyd (already in the repo: `etc/docker/proxyd/`) with
rate limiting; they double as the public-facing verifiers proving the
sequencer honest.

## Sizing rationale

- **Sequencer on local NVMe, not EBS.** Block building has a 2 s budget
  (250 ms for flashblocks) and does heavy random state reads; EBS adds
  ~0.5–1 ms per read vs ~0.1 ms local (measured on the devnet box: gp3 w_await
  2–7 ms under load). This is the only disk-critical box. Instance-store data
  is ephemeral, which is acceptable: the L2 chain is fully re-derivable from
  L1 batches, and an RPC node's EBS snapshot serves as a warm restore.
- **L1 node is "boring infra"**: geth+Nimbus steady-state IO is a few MB/s
  (measured); gp3 defaults suffice. 1 TB covers Sepolia ~400 GB + growth
  headroom. The currently syncing c7i.8xlarge (54.255) can be **repurposed as
  this box** at launch — its L1 data dir carries over, saving a 6-hour re-sync
  (move the EBS volume or keep the box as-is and only relocate devnet duties).
- **L2 disks start near-empty** (fresh genesis). At 2 s blocks and testnet
  load, expect tens of GB over the first months; 500 GB gives a year of slack.
- **RAM**: 64 GB on the sequencer (reth-based EL + rollup node + batcher);
  32 GB elsewhere (geth wants ~16, postgres/Blockscout wants 8–16).
- **No 24/7 GPU**: proof generation is on-demand (proven 2026-06-10 on a
  rented L40S: compressed ~9 min + Groth16). Rent g6e-class only for proving
  sessions; budget separately (~$2/h when used).

## Network / security groups

| Port | Box | Exposure |
|---|---|---|
| 30403/tcp+udp, 9100/tcp, 9101/udp | L1 node | public (L1 p2p) |
| L2 EL p2p + CL p2p (devnet-env values) | sequencer, RPC | public |
| 443 → proxyd → 8545 | RPC boxes | public via LB/nginx + rate limit |
| 443 → gateway, explorer, faucet | services box | public via nginx |
| 8645/8651/5152 (L1 RPC/authrpc/beacon) | L1 node | **private** (VPC/SG only) |
| Engine API 8551, metrics, postgres | all | private |

## Ops baseline

- EBS snapshots: nightly on L1 node + one RPC node (chain-data restore source)
  and services box (postgres). Sequencer needs no snapshot (re-derivable).
- Prometheus scrapes all boxes; alert on: head stall > 1 min, safe-head lag,
  batcher wallet balance (< 0.2 ETH), disk > 80%, L1 node sync distance.
- Batcher key funded from a monitored hot wallet; deployer key goes cold after
  launch.
- Sepolia blobs prune in ~18 days — if historical ZK proving is needed, point
  the prover at a blob archive (ethpandaops) instead of our own L1 node.

## Explicitly deferred to mainnet planning

- Sequencer HA (conductor + follower, leader transfer) — single sequencer is
  accepted testnet risk; an unplanned outage halts the chain but loses nothing.
- Fault proofs / permissionless proposing (testnet deploys permissioned).
- Multi-region RPC, CDN, DDoS posture beyond basic rate limiting.
- io2 / larger fleets — revisit with real traffic data from the testnet.
