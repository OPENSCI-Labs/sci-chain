# Sepolia L1 EL — Snapshot Import: REJECTED (decision record)

> **Status (2026-06-15): snapshot import abandoned. Continuing the in-progress reth
> staged sync on an expanded 1.5 TB disk.** This file was originally a plan to replace
> the slow full-execution staged sync of the Sepolia L1 EL (reth) with a `reth download`
> snapshot import. Step 0 (`--list`) proved no usable snapshot exists for Sepolia, so
> the plan is dead. Kept as a record so the snapshot path is not re-investigated from
> scratch.

Server: `54.255.70.252` (EC2 `c7i.8xlarge`, `i-066cbe9064d4cbfb5`). Affects **only the
L1 EL (reth)** — the CL (Nimbus) was already synced (`sync_distance=0/1`) throughout and
is left untouched.

## Why the snapshot path was rejected (verified 2026-06-15)

Both candidate sources failed when actually probed:

| Source | Finding |
|---|---|
| **reth official** (`snapshots.reth.rs`) | `reth download --chain sepolia --list` → `(no modular snapshots found)`. The index serves **mainnet only** (all entries are `reth-1-archive-stable-…`, chain 1, block ~25.3M). No Sepolia snapshot is published. |
| **PublicNode** (backup) | Has a Sepolia reth pruned snapshot but it is **473 GB** (not the 127.7 GB this doc originally assumed), it is **full-pruned (keeps all bodies), not the `--minimal` profile** the node runs, and the version label (`rethv0.9.1`) needs db-compat verification against the running reth 2.3.0. Not worth the risk. |

**Root cause of the slowness (the real lesson):** the box is running **reth 2.3.0
staged-sync (full re-execution from genesis)**, whereas `README.md`'s locked plan was
**EL = geth + `--syncmode=snap`** (snap downloads state from peers; hours, not days).
reth does **not** support snap sync — on reth the only fast path is a snapshot import,
and no Sepolia snapshot exists. So the deployment drifted from the README plan onto the
slow path. If a fast L1 is ever needed again, redeploy the EL as **geth snap** per the
README rather than chasing a reth snapshot.

## Why continuing reth is now fine (the disk crunch resolved)

The original fear was an imminent out-of-disk. That was the one-time **Bodies-stage**
download (14 G → 575 G in ~12 h on 06-13), which is **over**. Measured state on 06-15:

| Item | Value |
|---|---|
| `static_files` (bodies+headers) | 551 G — fully downloaded to tip; **not pruned until the pipeline reaches tip** |
| `db` (state) | 45 G — the only thing still growing, now at ~0.5–0.9 G/h |
| Execution stage | block ~5.0 M / 11.05 M (≈45% by block, **15% by gas**) |
| Disk | **1.5 TB** root EBS (`vol-0137cd731d40db8c4`), 686 G used, **767 G free** |

**Disk was expanded from ~1 TB to 1.5 TB on 2026-06-15** (EBS resize + growpart +
resize2fs all already applied; `nvme0n1p1` and ext4 both show 1.5 T). Projected
additional growth to tip is +150–250 G (db/state + changesets accumulating until the
tip-prune), so peak usage ≈ 850–940 G, leaving ~560 G free at the peak. After tip the
prune drops the 551 G of bodies and reth-data collapses to ~150–200 G (minimal =
state + headers). **No further expansion is needed.**

## What remains

- **ETA to tip: ~13–15 days** at the current rate (~500 k blocks/day; slower by gas as
  late Sepolia blocks are heavier). Expanding the disk solved "will it crash", not "how
  long" — there is no faster path on reth without a snapshot, and none exists for Sepolia.
- Monitor with `l1-node/sync-status.sh`. Caught-up check (from the box):
  ```bash
  curl -s -X POST -H 'Content-Type: application/json' \
    --data '{"jsonrpc":"2.0","method":"eth_syncing","params":[],"id":1}' \
    http://127.0.0.1:8645        # want: "result": false
  ```
- Once at head, the L1 is ready for `etc/scripts/devnet/setup-l2.sh` (op-deployer →
  SCI allocs → L2 stack) per `README.md`.

## Sources (snapshot investigation)

- reth download CLI: https://reth.rs/cli/reth/download/
- Reth official snapshots (mainnet only): https://snapshots.reth.rs/
- PublicNode snapshots: https://www.publicnode.com/snapshots
