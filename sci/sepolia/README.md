# SCI Chain on Sepolia (public testnet deployment)

Deployment of SCI Chain (L2, chain ID 42001) with **Sepolia as L1**, on the devnet
server `54.255.70.252`. Separate from `sci/devnet/`, which is the local Docker devnet
(self-hosted L1). Alloc patching scripts (`apply-sci-allocs.sh`,
`apply-predeploy-allocs.sh`) are shared — they live in `sci/devnet/` and are referenced
from here, not copied.

## Layout

```
sci/sepolia/
├── l1-node/   ← Sepolia L1 node: geth (EL) + Nimbus (CL) compose/config
├── deploy/    ← op-deployer intent + setup-l2 invocation env for L1=Sepolia
└── .gitignore ← real keys / jwt / .env never enter the repo
```

## Plan summary (locked 2026-06-11)

- **EL = geth**: `--syncmode=snap --state.scheme=path` (~300-400 GB steady state on
  Sepolia; body history is the bulk).
- **CL = Nimbus**: checkpoint sync + `--history=prune` + `--backfill=false` — a fresh
  L2 only needs L1 forward from deploy time, not pre-deploy history (minutes to head,
  tens of GB).
- Checkpoint sync URL: `https://sepolia.checkpoint-sync.ethpandaops.io`
- Ports (clash-free with the devnet stack on the same box): EL http `8645`,
  authrpc `8651`, p2p `30403`; CL http `5152`, p2p `9100/9101`; shared `jwt.hex`.
- After L1 sync: `etc/scripts/devnet/setup-l2.sh` with `L1_RPC_URL=http://localhost:8645`,
  `L1_BEACON=http://localhost:5152`, `L1_CHAIN_ID=11155111`, `L2_CHAIN_ID=42001` →
  op-deployer deploys L1 contracts (permissioned, no fault proofs) → apply SCI allocs →
  start the L2 stack (compose minus `l1-*` services).

## Prerequisites

- Sepolia ETH: deployer ~0.1–0.5 + batcher ongoing blob fees (faucets ok).
- Real private keys via `.env` (gitignored) — do NOT use the devnet test-junk mnemonic.

## Caveats

- Sepolia blobs prune after ~18 days — proving old L2 ranges later needs a blob archive.
- L1 escape-hatch Tier 2 gets its first real-OptimismPortal e2e test here.
