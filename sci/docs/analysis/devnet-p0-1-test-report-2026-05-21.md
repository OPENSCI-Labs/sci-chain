---
title: "P0-1 Keychain Devnet 集成测试报告"
date: "2026-05-21"
branch: "feat/p0-1-keychain"
commit_at_start: "ef4914ea8"
status: "T1-T6 全部通过；T7 长跑可选未跑；T8 阻塞于 Heath SCIAgentDelegator"
---

# P0-1 Keychain Devnet 集成测试报告

本文档记录 2026-05-20 ~ 05-21 在 `ubuntu@54.255.70.252` devnet 服务器上对
P0-1 Keychain 精灵的完整集成测试过程、踩过的所有坑、最终解决方案、对运行环境做的
所有持久化改动，以及对后续测试系统的建议。

阅读它的人应该能：
- 知道当前 devnet 上 SCI fork 已验证哪些行为
- 复用文中"环境改动清单"在新机器上一键重建相同的测试环境
- 看懂为什么我们在 `sci/devnet/` 加了那两个文件
- 知道下次跑 P0-1 之前要做什么、不要做什么

---

## 0. Executive Summary

- **测试结论：P0-1 Keychain 精灵在 devnet 上端到端工作正常。** T1–T6 全部通过、0 panic、链稳定出块。
- **三个非平凡阻塞点被识别并解决**（按发现顺序）：
  1. handoff doc 用 `dev` profile build SCI image → reth 在 rayon worker pool
     里持续触发 panic，每 2–6s 一次。**必须改用 `release` profile**。
  2. SCI 精灵地址（`0xAAAA...0000`、`0xAAAA...0001`）**未在 devnet genesis alloc 中**，
     EIP-161 把 precompile 账户当 empty account GC，sstore 在 tx 结束时被静默丢弃。
     **解决：genesis alloc 给这两个地址加 `code:"0xef"` 占位**（沿用 Tempo 设计）。
  3. genesis alloc patch 改变 L2 genesis state root → genesis hash，但 op-deployer
     生成的 `rollup.json` 仍存旧 hash，CL 永远卡在 `AwaitingELSyncCompletion`。
     **解决：同步 patch `rollup.json` + `rollup-conductor.json` 的 `.genesis.l2.hash`。**
- **image tag 规范立起来了**：`:local` = base 原版 / `:sci` = SCI release / `:sci-dev-broken`
  = 已知坏的取证镜像 / `:base-rollback` = 防 buildx GC 的永久 tag。Hot-swap 走
  `sci/devnet/docker-compose.sci.yml` compose override，rollback = 去掉那个 `-f`。
- **新增/改动文件**全部在 `sci/devnet/` 内（无 Base 文件污染），但 devnet **运行时**
  的 `.devnet/l2/configs/{genesis,rollup,rollup-conductor}.json` 是 op-deployer 一次性生成、
  需要每次重启 devnet 都应用一次 patch。我们把这一步沉淀成 `apply-sci-allocs.sh`，
  尚未把 hash patch 也脚本化（见 §8 建议）。
- **当前 devnet 状态**：base-client + base-builder 跑在 `base-reth-node:sci` /
  `base-builder:sci`，链 ID 42001，block 600+，0 panic，所有 SCI precompile 测试通过。

---

## 1. 测试环境

| 项 | 值 |
|---|---|
| 远端服务器 | `ubuntu@54.255.70.252` |
| SCI 工作目录 | `~/sci-dev/sci-chain/`（fork: base-v0.8 + sci/） |
| 运行栈所在目录 | `~/sci-dev/base-v0.8/`（pure base-v0.8 + 3 ops patches） |
| Tempo 参考目录 | `~/sci-dev/tempo/`（仅本地有，远端无） |
| Blockscout | `~/sci-dev/blockscout/`（独立 compose project，不受 devnet down 影响） |
| Chain ID | 42001 |
| SCI 提交 | `ef4914ea8 sci: port keychain precompile and wire pre-execution hook` |
| Reth 版本 | `v1.11.4` (tag `2ac58a25`) |
| Rust toolchain | 1.93.1 / edition 2024 |

---

## 2. 测试分阶段过程

### Phase A — 远端编译 + 单测（无风险）

```bash
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/sci-chain && \
  cargo check -p sci-precompiles -p sci-precompile-abi -p base-common-evm && \
  cargo test -p sci-precompiles --lib && \
  cargo test -p sci-precompiles --test hook_e2e'
```

**结果**：
- `cargo check` 通过（41.86s）
- `cargo test --lib`: **307 passed; 0 failed; 1 ignored** ✓
- `cargo test --test hook_e2e`: **14 passed; 0 failed** ✓
- 与本地完全一致。远端 toolchain 没有惊喜。

### Phase B — Build SCI Docker Image

**handoff doc 写的版本**（错的，留作教训）：

```bash
just devnet build-image client dev   # 第二参数 dev = profile
```

`dev` profile = unoptimized debug build。SCI image 起来后 base-client 进入持续 panic
循环：

```
ERROR panic in worker pool thread msg=wait_cloned must not be called from a rayon worker thread
WARN  State root task timed out, spawning sequential fallback timeout=1s
```

每 2–6s 一次，1 分钟内累计 1680+ 次。链靠 state-root 顺序计算 fallback 维持出块，
但 latency 偏高、行为不可信，不能用来跑功能测试。

panic 在 `reth-e231042ee7db3fb7/2ac58a2/crates/chain-state/src/deferred_trie.rs:316` —
完全是 reth 内部代码，跟我们 SCI 改的 EVM 路径无关。

**根因**：`etc/docker/docker-bake.hcl` 默认 `PROFILE=release`，
`Dockerfile.rust-services` 默认 `ARG PROFILE=release`。我们显式传 `dev` 覆盖了默认。
debug build 触发了 reth 的 rayon worker pool 异常路径（怀疑是 `debug_assert!` 或
rayon 在 debug 下的不同线程行为）。

**正确做法**：

```bash
just devnet build-image client release && \
just devnet build-image builder release
```

release build 后切到 SCI image，**panic count = 0**。

base-builder 同样要 build SCI release：因为它通过 `base-common-evm` workspace 依赖
间接拉到我们改过的 `factory.rs` + `SciHandler`，必须和 base-client 保持一致，否则
builder 让过的 tx 跟 client 期望的 hook 行为不一致 → 链 stall。

### Phase C — Image Tag 隔离方案

**handoff doc 默认的做法**：build 完直接覆盖 `base-reth-node:local`。这有两个问题：

1. **失去 rollback 目标**：`:local` 一旦被 SCI build 覆盖，原 base image 失去 tag。
   `docker compose --force-recreate` 后，旧 untagged image 在下次 build 触发 buildx GC
   时被自动清掉（我们实测验证：base 老 image ID `95d87ea23b...` 完全消失，
   `docker images -a` 找不到，没法 retag 回来）。
2. **没法 A/B 对比**：同一时间只能有一个 image 占用 `:local` tag。

**采用的隔离方案**：

| Tag | 指向 | 用途 |
|---|---|---|
| `base-reth-node:local` / `base-builder:local` | base-only release build | 默认 rollback 目标（pure base） |
| `:base-rollback` | base-only release build（永久 tag） | 防 buildx GC，灾备 |
| `:sci` | SCI release build | SCI 测试 swap 目标 |
| `:sci-dev-broken` | 当前已知坏的 dev-profile SCI build | 取证保留，不要跑 |

**tag dance 顺序**（关键：build 完必须立刻打**额外**永久 tag，否则下次 build 会孤儿化它）：

```bash
# 1. 从 ~/sci-dev/base-v0.8/ build base-only release，立即打 :base-rollback
cd ~/sci-dev/base-v0.8 && just devnet build-image client release && \
  docker tag base-reth-node:local base-reth-node:base-rollback
cd ~/sci-dev/base-v0.8 && just devnet build-image builder release && \
  docker tag base-builder:local base-builder:base-rollback

# 2. 从 ~/sci-dev/sci-chain/ build SCI release，立即打 :sci，把 :local 还回 base-only
cd ~/sci-dev/sci-chain && just devnet build-image client release && \
  docker tag base-reth-node:local base-reth-node:sci && \
  docker tag base-reth-node:base-rollback base-reth-node:local
cd ~/sci-dev/sci-chain && just devnet build-image builder release && \
  docker tag base-builder:local base-builder:sci && \
  docker tag base-builder:base-rollback base-builder:local
```

最终 `docker images` 状态（验证脚本）：

```
base-reth-node:local           = base-only release ID
base-reth-node:base-rollback   = same ID (双 tag)
base-reth-node:sci             = SCI release ID
base-reth-node:sci-dev-broken  = dev-profile SCI ID（forensic）
base-builder:*                 同上四组
```

### Phase D — Hot-swap with Compose Override

为了让 base-v0.8 stack 用 SCI image 而不污染 base compose 文件，新增：

**文件**：`sci/devnet/docker-compose.sci.yml`

```yaml
services:
  base-client:
    image: base-reth-node:sci
  base-builder:
    image: base-builder:sci
```

Hot-swap 命令：

```bash
cd ~/sci-dev/base-v0.8
docker compose \
  --env-file etc/docker/devnet-env \
  -f etc/docker/docker-compose.yml \
  -f ~/sci-dev/sci-chain/sci/devnet/docker-compose.sci.yml \
  up -d --no-build --no-deps --force-recreate base-client base-builder
```

Rollback 路径就是去掉第二个 `-f`，重跑——服务回到 `:local` = base-only。

### Phase E — 触发 Genesis Bug 与修复

切到 release-profile SCI image 后链稳了，开始跑 T1-T6 功能测试。前几个全过，
但 **T4（authorizeKey 端到端）发现 storage 不持久化**：

```
authorizeKey tx -> status=0x1, gasUsed=51,794
KeyAuthorized event 正常 emit
但: cast call getKey(ROOT, SESSION_KEY) 返回全零
   cast storage 0xAAAA...0000 <slot> 也全零
```

#### 根因：SCI 精灵地址没在 genesis alloc，被 EIP-161 GC

revm 把 `0xAAAA...0000` 当成 "newly created empty account"（因为它不在 genesis alloc
里，没有 code，没有 balance，没有 nonce）。tx 结束时 EIP-161 garbage collect 把整个账户
（包括我们 sstore 写入的 storage）丢掉。Event log 是单独的存储通道，不受 account
GC 影响，所以 event 出来但 storage 丢了。

Tempo 的 `dev.json` 用一个 1-byte INVALID opcode 占位解决：

```json
"0xaaaaaaaa00000000000000000000000000000000": {
  "nonce": "0x0",
  "balance": "0x0",
  "code": "0xef"
}
```

`0xef` = INVALID opcode：
- 让账户 non-empty，EIP-161 不 GC ✓
- 直接 call 该地址会立即 revert（而不是 STOP 静默成功） ✓
- precompile dispatch 在 EVM 内的优先级高于 code 执行，所以正常 precompile 调用不受 0xef 影响 ✓

#### 修复方案：新增 `sci/devnet/sci-allocs.json` + `apply-sci-allocs.sh`

**文件**：`sci/devnet/sci-allocs.json`

```json
{
  "0xaaaaaaaa00000000000000000000000000000000": {
    "nonce": "0x0",
    "balance": "0x0",
    "code": "0xef"
  },
  "0xaaaaaaaa00000000000000000000000000000001": {
    "nonce": "0x0",
    "balance": "0x0",
    "code": "0xef"
  }
}
```

**文件**：`sci/devnet/apply-sci-allocs.sh` — jq 合并脚本：

```bash
bash ~/sci-dev/sci-chain/sci/devnet/apply-sci-allocs.sh \
  /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/genesis.json
```

#### 二次问题：genesis hash 漂移与 rollup.json 失同步

加了 alloc 之后 L2 genesis state root 变了 → genesis block hash 也变了
（从 `0x0e5826...` 变成 `0x9dff0d...`）。但 op-deployer 已经在
`rollup.json` 和 `rollup-conductor.json` 写了旧 hash。CL 启动时用 rollup.json
的旧 hash 跟 EL 实际算出的新 hash 对比，永远不一致 → 永远卡
`AwaitingELSyncCompletion`，链不出块。

修复：apply genesis patch 后，再 patch rollup config 的 `.genesis.l2.hash`：

```bash
NEW_HASH=$(cast block 0 --rpc-url http://localhost:8545 --json | jq -r .hash)
for f in rollup.json rollup-conductor.json; do
  sudo jq --arg h "$NEW_HASH" '.genesis.l2.hash = $h' \
    /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/$f \
    > /tmp/p && sudo mv /tmp/p /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/$f
done
# 然后 docker compose --force-recreate base-client-cl base-builder-cl
```

#### 三次问题：base-client flashblocks 流断裂后没自愈

base-client/base-builder 通过 WebSocket flashblocks 流同步。中间出过短暂断流，
base-client 卡在某个 block 不再前进，但 base-builder 持续出块。日志：

```
ERROR Received non-zero index Flashblock for new block, zeroing Flashblocks until we receive a base Flashblock
```

修复：`docker compose ... --force-recreate base-client` — 让它重启重新订阅 flashblocks。
之后 client 立刻追上 builder。

---

## 3. 最终测试结果

### 功能测试 T1-T6

环境变量：

```bash
RPC=http://localhost:8545
ROOT_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
ROOT_ADDR=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
SESSION_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a
SESSION_KEY=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC
BYSTANDER=0x90F79bf6EB2c4f870365E785982E1f101E93b906
KEYCHAIN=0xAAAAAAAA00000000000000000000000000000000
SCI_AGENT_STATE=0xAAAAAAAA00000000000000000000000000000001
```

| 测试 | 命令 | 期望 | 实际 | 状态 |
|---|---|---|---|---|
| **T1** chain baseline | `cast chain-id` / `block-number` / `balance $ROOT_ADDR` | 42001 / growing / non-zero | 42001 / 600+ growing / 10000 ETH | ✅ |
| **T2** keychain reachable | `cast call $KEYCHAIN "getKey(address,address)((uint8,address,uint64,bool,bool))" $ROOT_ADDR $SESSION_KEY` | `(0,0x0,0,false,false)` for unauth pair | `(0,0x0,0,false,false)` | ✅ |
| **T2** SciAgentState reachable | `cast call $SCI_AGENT_STATE "isTripped(address)(bool)" $SESSION_KEY` | `false` | `false` | ✅ |
| **T2.5** keychain has code | `cast code $KEYCHAIN` | `0xef` (alloc patch) | `0xef` | ✅ |
| **T3** ETH transfer no regression | `cast send $BYSTANDER --value 0.01ether` | status=0x1, gas≈21000 | status=0x1, gas=21000 (0x5208) | ✅ |
| **T4** authorizeKey + getKey | `cast send authorizeKey(...)` → `cast call getKey(...)` | `(0,SESSION_KEY,u64::MAX,false,false)` | `(0,0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC,18446744073709551615,false,false)` | ✅ |
| **T4** duplicate authorize | second `authorizeKey` for same pair | revert `KeyAlreadyExists` (0xaa1ba2f8) | revert `KeyAlreadyExists` ✓ | ✅ |
| **T5** SciAgentState ACL | `cast send tripKey(...)` from non-CB | revert `Unauthorized` (0x82b42900) | revert `0x82b42900` ✓ | ✅ |
| **T6.1** 7702 to non-SCI delegate | `cast send --auth $AUTH ...` w/ `RANDOM_DELEGATE=0x...beef` | status=0x1 (hook fast-paths) | status=0x1, gas=0xb3b0 | ✅ |
| **T6.2** normal transfer post-7702 | `cast send $BYSTANDER --value 0.001ether` from delegated key | status=0x1, gas=21000 (fast-path) | status=0x1, gas=0x5208 | ✅ |

### 健康指标

- **panic count（自 SCI release image 启动以来）**：`docker logs base-client \| grep -c "panicked at"` = **0**
- **panic count base-builder**：**0**
- **block production**：稳定 2s/块
- **container health**：base-client / base-builder / base-client-cl / base-builder-cl 全部 `healthy`
- **base-batcher**：稳定运行（不再 restart loop）

### 未跑

- **T7** 长跑稳定性（tmux 后台 1 小时连发 tx）：可选，未跑
- **T8** 完整 agent-tx 端到端（SCIAgentDelegator → 多 call batch → keychain hook → R1
  semantics 验证）：**阻塞于 Heath 的 `SCIAgentDelegator.sol` 部署到 `0xCCCC...01`**。
  这部分需要再加 genesis alloc（参考 Tempo 的 `0xcccccccc...` 条目），且需要
  EIP-7702 set-code 到那个地址。在 Heath 落地之前不在本次测试范围。

---

## 4. Devnet 运行时环境改动清单

测试过程中对远端 `~/sci-dev/base-v0.8/.devnet/` 下的运行时文件做了以下持久化改动，
若 devnet 重启需要重新应用：

| 文件路径 | 改动 | 备份 |
|---|---|---|
| `.devnet/l2/configs/genesis.json` | `.alloc` 加 `0xAAAA...0000` 和 `0xAAAA...0001`（each: `{nonce:"0x0", balance:"0x0", code:"0xef"}`） | `genesis.json.pre-sci` |
| `.devnet/l2/configs/rollup.json` | `.genesis.l2.hash` = 新 genesis hash（重新算） | `rollup.json.pre-sci` |
| `.devnet/l2/configs/rollup-conductor.json` | `.genesis.l2.hash` = 新 genesis hash | `rollup-conductor.json.pre-sci` |
| `.devnet/l2/client/`, `.devnet/l2/builder/`, `.devnet/l2/{client,builder}-cl/` | **被 wipe** 一次（在 patch 后让 EL/CL 从 fresh state 启动新 chain） | — |

Docker image tags 新增：

| Tag | Image ID（远端实测） |
|---|---|
| `base-reth-node:sci` | `87d4da1e4c22` (release SCI build) |
| `base-builder:sci` | `668458cff759` (release SCI build) |
| `base-reth-node:base-rollback` | `402115ddcdff` (release base-only) |
| `base-builder:base-rollback` | `1bd3cba28711` (release base-only) |
| `base-reth-node:sci-dev-broken` | `32c8455a3ca8` (dev-profile SCI) |
| `base-builder:sci-dev-broken` | `9ba8bda558d6` (dev-profile SCI) |
| `base-reth-node:local` | = base-rollback ID（双 tag） |
| `base-builder:local` | = base-rollback ID |

Running container layout：

| Container | Image | Notes |
|---|---|---|
| `base-client` | `base-reth-node:sci` | SCI keychain precompile 安装在 0xAAAA...0000 / 0xAAAA...0001 |
| `base-builder` | `base-builder:sci` | Same |
| `base-client-cl`, `base-builder-cl` | `base-consensus:local` | 不需要 SCI 改动 |
| `base-batcher` | `base-batcher:local` | 不需要 SCI 改动 |
| L1 stack (`l1-el`, `l1-cl`, `l1-vc`, `setup-l1`) | 原 base image | 完全不需要 SCI 改动 |

---

## 5. 仓库内新增文件（`sci/devnet/`）

| 文件 | 用途 |
|---|---|
| `sci/devnet/docker-compose.sci.yml` | Compose override，把 `base-client.image` / `base-builder.image` 指向 `:sci` |
| `sci/devnet/sci-allocs.json` | SCI 精灵 genesis alloc 数据（keychain + SciAgentState） |
| `sci/devnet/apply-sci-allocs.sh` | jq 合并脚本，把 sci-allocs.json 合并到 op-deployer 生成的 genesis.json |

---

## 6. 完整的从零起 devnet 工作流（应用所有 lessons）

把所有踩坑总结成一套 step-by-step 流程，下次新机器 / 全量重启 devnet 应用：

```bash
# === 前置 ===
# 在 ~/sci-dev/base-v0.8/（pure base 克隆）build base-only release images
cd ~/sci-dev/base-v0.8
just devnet build-image client release
docker tag base-reth-node:local base-reth-node:base-rollback
just devnet build-image builder release
docker tag base-builder:local base-builder:base-rollback

# 在 ~/sci-dev/sci-chain/（SCI fork）build SCI release images
cd ~/sci-dev/sci-chain
just devnet build-image client release
docker tag base-reth-node:local base-reth-node:sci
docker tag base-reth-node:base-rollback base-reth-node:local   # 还原 :local
just devnet build-image builder release
docker tag base-builder:local base-builder:sci
docker tag base-builder:base-rollback base-builder:local

# === Step 1: stop + wipe devnet ===
cd ~/sci-dev/base-v0.8
just devnet down  # 注意：会 rm -rf .devnet/*；blockscout 不受影响

# === Step 2: 启 L1 全栈 ===
docker compose --env-file etc/docker/devnet-env -f etc/docker/docker-compose.yml \
  up -d l1-el l1-cl l1-vc setup-l1
# 等 L1 开始出块（4s/块），cast block-number --rpc-url http://localhost:4545

# === Step 3: 启 setup-l2，生成 genesis ===
docker compose --env-file etc/docker/devnet-env -f etc/docker/docker-compose.yml \
  up -d setup-l2
# 等到 docker inspect setup-l2 显示 "exited|exit=0"

# === Step 4: apply SCI alloc patch 到 genesis.json ===
sudo bash ~/sci-dev/sci-chain/sci/devnet/apply-sci-allocs.sh \
  /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/genesis.json

# === Step 5: 启 L2 服务（SCI override），但用 --no-deps 不让 setup-l2 再跑 ===
docker compose \
  --env-file etc/docker/devnet-env \
  -f etc/docker/docker-compose.yml \
  -f ~/sci-dev/sci-chain/sci/devnet/docker-compose.sci.yml \
  up -d --no-build --no-deps base-client base-builder base-client-cl base-builder-cl base-batcher
# base-client + base-builder 现在用 :sci image，读 patched genesis，计算新 genesis hash

# === Step 6: 用新 genesis hash patch rollup.json 和 rollup-conductor.json ===
NEW_HASH=$(cast block 0 --rpc-url http://localhost:8545 --json | jq -r .hash)
for f in rollup.json rollup-conductor.json; do
  sudo cp -p /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/$f \
    /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/$f.pre-sci
  sudo bash -c "jq --arg h \"$NEW_HASH\" '.genesis.l2.hash = \$h' \
    /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/$f > /tmp/p \
    && mv /tmp/p /home/ubuntu/sci-dev/base-v0.8/.devnet/l2/configs/$f"
done

# === Step 7: force-recreate CL 让它重读 rollup.json ===
docker compose --env-file etc/docker/devnet-env \
  -f etc/docker/docker-compose.yml \
  -f ~/sci-dev/sci-chain/sci/devnet/docker-compose.sci.yml \
  up -d --no-build --no-deps --force-recreate base-client-cl base-builder-cl base-batcher

# === Step 8: 若 base-client 卡在初始 block，force-recreate 让它重订 flashblocks ===
docker compose --env-file etc/docker/devnet-env \
  -f etc/docker/docker-compose.yml \
  -f ~/sci-dev/sci-chain/sci/devnet/docker-compose.sci.yml \
  up -d --no-build --no-deps --force-recreate base-client

# === Verify ===
cast block-number --rpc-url http://localhost:8545      # 应增长
cast code 0xAAAAAAAA00000000000000000000000000000000 --rpc-url http://localhost:8545  # 应为 0xef
docker logs base-client 2>&1 | grep -c "panicked at"   # 应为 0
```

---

## 7. Rollback 与故障回滚

### 完全 rollback 到 pure base（无 SCI 改动）

```bash
cd ~/sci-dev/base-v0.8
docker compose \
  --env-file etc/docker/devnet-env \
  -f etc/docker/docker-compose.yml \
  up -d --no-build --no-deps --force-recreate base-client base-builder
# 没传 sci override，会用 :local（= base-only release），SCI precompile 失效
```

### 回到 broken dev image（debug，禁止生产）

```bash
docker compose ... --force-recreate ...    # 此前用 sci-override 跑过 :sci-dev-broken
# 一般用不上，仅 forensic 时复现
```

### 全量 rebuild base-only

如果不慎丢失 base image，从 `~/sci-dev/base-v0.8/` 重新 build：

```bash
cd ~/sci-dev/base-v0.8
just devnet build-image client release
docker tag base-reth-node:local base-reth-node:base-rollback   # 立即打永久 tag
just devnet build-image builder release
docker tag base-builder:local base-builder:base-rollback
```

---

## 8. 后续测试系统建议

基于这次踩的所有坑，给出对"更好的测试系统"的建议。**优先级 1（must）** 是不做就
下次还会踩，**优先级 2（should）** 是建议但可推迟。

### 优先级 1（必做）

1. **修正 handoff doc `dev` → `release` profile**。
   - 当前 `sci/docs/analysis/devnet-testing-handoff.md` 里 `just devnet build-image client dev`
     需要全部改成 `release`。
   - 顺便补一行说明：dev profile 会触发 reth rayon panic，不能用于跑功能测试。

2. **修正 handoff doc `getKey` ABI 签名**。
   - 当前文档：`getKey(address,address)(uint8,uint64,bool,bool)` ← 错的（缺 `address keyId`）
   - 正确：`getKey(address,address)((uint8,address,uint64,bool,bool))` ← struct return wrap
   - 顺便修 `RANDOM_DELEGATE=0x00000000000000000000000000000000000DEADBE` 是 41 个 hex char。
     正确：`0x000000000000000000000000000000000000bEEF` 或 `0x00000000000000000000000000000000000DeAdBE`（去掉一个 0）。

3. **把 genesis alloc + rollup hash patch 串成一个一键脚本**。
   - 当前：`apply-sci-allocs.sh` 只 patch genesis。rollup hash 是手工 jq 改的。
   - 应该：`sci/devnet/apply-sci-patches.sh` 包含两步：
     a. 合并 `sci-allocs.json` 进 genesis.json
     b. 等 base-client 起来后查 `cast block 0` 的新 hash，patch `rollup.json` + `rollup-conductor.json`
   - 这样下次 devnet down + up 只需要：起 setup-l2 → 跑这个脚本 → 起 client/builder。

4. **`sci/devnet/` 加一个 README**，列：
   - "新机器从零起 devnet" 的步骤（从 §6 抄一份）
   - "SCI 精灵地址清单 + 为什么每个都需要 alloc"
   - "rollback 命令"
   - "已知问题：base-client 启动可能卡在 flashblocks，--force-recreate 即可"

### 优先级 2（应该做）

5. **把 `sci/devnet/sci-allocs.json` 加进 op-deployer 的 intent 文件**。
   - op-deployer 支持 intent file 配 alloc。如果能在 generate 时就把 SCI alloc 加进去，
     就不需要 post-generation patch，也避免 rollup.json hash 漂移（hash 直接是带 SCI
     alloc 的）。
   - 但这条需要研究 op-deployer 的 intent schema，且可能要 base-v0.8 改 `setup-l2.sh`
     —— 触碰 Base 文件违反 CLAUDE.md Rule #1。要在权衡后决定。

6. **写一个 `cargo test` 等价的 devnet smoke test**。
   - 即一个 Rust 程序，对 devnet RPC 跑 T1-T6 + 断言，集成进 `just devnet smoke-test`。
   - 这样 PR / CI 可以自动验证 "SCI image 在 devnet 上行为符合预期"。
   - 当前 T1-T6 是手工 bash，每次手贴变量，易出错。

7. **打开 `evm-bridge-tests` feature 的 CI 跑一遍**。
   - CLAUDE.md 提到 `storage/evm.rs` integration tests gated behind `evm-bridge-tests`
     feature（off by default）。**就是这次 devnet 阶段才暴露的 EIP-161 GC 问题，如果
     `evm-bridge-tests` 跑了，应该在 cargo test 阶段就能 catch**。
   - 建议：CI 跑 `cargo test -p sci-precompiles --features evm-bridge-tests`，
     作为 PR-required check。

8. **确定 SCI 主网 genesis 模版**。
   - 当前我们只是在 devnet 局部 patch genesis。SCI 主网上线时需要正式 chainspec
     包含 SCI precompile alloc。现在还没有 mainnet genesis 模版。
   - 建议在 `sci/devnet/sci-allocs.json` 旁加一份 `sci/chainspec/sci-mainnet-allocs.json`
     作为参考起点，且写明哪些 alloc 是"协议必须"vs"测试方便"。

9. **设法摆脱 `--force-recreate base-client` 这一步**。
   - 当前我们必须手动 force-recreate base-client 让它重连 flashblocks。这是个
     workaround。理想是 base-client 自己检测到 flashblocks 流断了就自动重连。
   - 这条已经超出 SCI scope，是 Base 上游 bug。可以提个 issue 给 Base 维护方。

10. **把 SCI image tag 规范写到 CLAUDE.md**。
    - 现在 `:local` / `:sci` / `:sci-dev-broken` / `:base-rollback` 的约定散落在
      memory 和这份报告里。值得在 CLAUDE.md "How the Keychain Precompile Is Wired"
      下面加一个 "Devnet Image Tag Convention" 子节，避免后人重蹈"覆盖 :local 失去
      rollback 目标"的覆辙。

### 已经覆盖、不需要新工作

- **release vs dev profile**：已记入 [[project-devnet-image-tags]] memory，下次不会再错。
- **EIP-161 GC for precompile addresses**：已记入 [[project-devnet-genesis-alloc-gap]]，
  并且 `sci/devnet/sci-allocs.json` 解决了这个问题。
- **Hot-swap pattern**：已经走通且写在 docker-compose.sci.yml 文件头注释里。

---

## 9. 阻塞项

- **T8 完整 agent-tx 闭环**：依赖 Heath 的 `SCIAgentDelegator.sol` 部署到
  `0xCCCC...01`，且需要给该地址加 genesis alloc（参考 Tempo 的 `0xcccccccc...` 条目），
  且 root EOA 需要做 EIP-7702 set-code 指向它。这部分到 Heath 落地后才能跑。
- **强 R1 真实场景验证**（hook 通过 → body revert → quota 未扣）：同样阻塞于
  SCIAgentDelegator 落地。Rust crate 里 14 个 `hook_e2e` 测试用 InMemoryDB 覆盖了
  这些路径，devnet 层只需 smoke test。

---

## 10. 一句话总结

P0-1 Keychain 精灵在 devnet 上端到端通了，但要走通需要：
**release profile 的 SCI image** + **SCI 精灵地址在 genesis alloc 里加 `code:"0xef"`**
+ **`rollup.json` 的 genesis hash 跟着 alloc 改动同步更新**。三件事缺一不可。
本次测试已把这三件事的工程化路径沉淀到 `sci/devnet/` 目录下，下次复现只需走 §6 流程。
