---
title: "P0-1 Keychain Devnet 集成测试 — Handoff 备注"
date: "2026-05-20"
status: "上传完成，等下一步执行"
---

# Where We Paused (2026-05-20 晚)

代码已 push 到 `origin/feat/p0-1-keychain` 并 rsync 上传到 devnet 服务器
`~/sci-dev/sci-chain/`。**下一步是 Phase A 远端 cargo check + 单测。**

## 服务器目录关系（必须懂）

| 路径 | 是什么 | 状态 |
|---|---|---|
| `ubuntu@54.255.70.252:~/sci-dev/base-v0.8/` | **运行中的 devnet stack**（docker compose 在跑 base-client / base-builder / l1-* / bs-* / jaeger / grafana） | 不要碰 |
| `ubuntu@54.255.70.252:~/sci-dev/sci-chain/` | **SCI fork 的 staging 区**（base-v0.8 同步 + 我们 rsync 上去的 SCI 代码） | 在这里 build 镜像 |

两个目录都有相同的 3 个 **ops patches（uncommitted 在 working tree）**，不要污染我们的 fork：

| Patch | 文件 | 性质 |
|---|---|---|
| Chain ID 42001 | `etc/docker/devnet-env` | **✅ 已经在我们 fork** |
| Jaeger badger storage | `etc/docker/docker-compose.yml` | ❌ ops only |
| `x-logging` 锚点 / log rotation | `etc/docker/docker-compose.yml` + `blockscout/docker-compose.yml` | ❌ ops only |

## 不可逾越的红线（CLAUDE.md 写的）

1. **不要 `just devnet up`** —— 它先跑 `down` 然后 `rm -rf .devnet/*`，链状态、Blockscout 历史全没
2. **不要 `docker compose down`** —— 同上
3. **不要在容器里 `docker exec` 改文件** —— 重启即失

**正确做法 = hot-swap**：build 新镜像（`:local` tag）→ 用
`docker compose ... up -d --no-deps --force-recreate <service>` 只重建一个 container。
其他 container 不动，链状态不丢，block 暂停 ~5 秒后恢复。

---

# 明天接着干 — 按顺序

## Phase A：远端编译 + 单测（无风险，~10 min）

```bash
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/sci-chain && \
  export PATH=$HOME/.cargo/bin:/usr/local/go/bin:/usr/local/bin:$PATH && \
  cargo check -p sci-precompiles -p sci-precompile-abi -p base-common-evm 2>&1 | tail -5'
```

期望：`Finished `dev` profile`，无 error。

```bash
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/sci-chain && \
  export PATH=$HOME/.cargo/bin:$PATH && \
  cargo test -p sci-precompiles --lib 2>&1 | tail -3 && \
  echo "---" && \
  cargo test -p sci-precompiles --test hook_e2e 2>&1 | tail -3'
```

期望：
- `307 passed; 0 failed; 1 ignored`（lib）
- `14 passed; 0 failed`（hook_e2e）

如果失败：很可能远端 toolchain 跟本地不同，先 `rustup show` 对一下版本（应该是 1.93.1）。

## Phase B：build SCI 镜像（中等风险，~3-10 min 增量）

我们 SCI 改动只动 EVM 执行路径（`crates/common/evm/*` + `sci/crates/precompiles/*`），需要重建的镜像：

```bash
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/sci-chain && \
  export PATH=$HOME/.foundry/bin:/usr/local/go/bin:$HOME/.cargo/bin:$PATH && \
  just devnet build-image client dev 2>&1 | tail -10'
```

**先确认 `just devnet build-image` 支持哪些 target**：
```bash
ssh ubuntu@54.255.70.252 'cat ~/sci-dev/sci-chain/Justfile | grep -A 1 "build-image"'
```

需要重建的可能不止 `client`：
- `client` → `base-reth-node:local`（base-client 用，跑 EVM execution）
- 可能还要：`builder` → `base-builder:local`（base-builder 用，也跑 EVM execution）

**`:local` tag 会被覆盖**。运行中的 base-v0.8 stack 暂时还用着旧 `:local`（Docker 保留运行中的 image ID，tag 重指不影响已运行的 container）。所以 B 跑挂了不会立刻影响 devnet。

期望：images 出现，体积合理：
```bash
ssh ubuntu@54.255.70.252 'docker images | grep -E "base-reth-node|base-builder|base-consensus"'
```

## Phase D：hot-swap 进运行 devnet（关键一步）

**只在 Phase B 成功后跑。**

```bash
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/base-v0.8 && \
  docker compose \
    --env-file etc/docker/devnet-env \
    -f etc/docker/docker-compose.yml \
    up -d --no-build --no-deps --force-recreate base-client'
```

如果 `base-builder` 也走 EVM execution（要看 Dockerfile），同样 swap：
```bash
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/base-v0.8 && \
  docker compose \
    --env-file etc/docker/devnet-env \
    -f etc/docker/docker-compose.yml \
    up -d --no-build --no-deps --force-recreate base-builder'
```

立刻验证链没死：
```bash
ssh ubuntu@54.255.70.252 'export PATH=$HOME/.foundry/bin:$PATH && \
  cast block-number --rpc-url http://localhost:8545; sleep 4; \
  cast block-number --rpc-url http://localhost:8545'
```

期望：两次输出 block 号，第二次比第一次大 ≥1。

看新容器跑的什么镜像：
```bash
ssh ubuntu@54.255.70.252 'docker inspect base-client --format "{{.Image}} created={{.Created}}"'
```

期望：image ID 是刚才 build 出来的新的（不是旧 ID）。

## Phase E：跑功能测试（D 成功后，~30 min）

变量准备：
```bash
RPC=http://54.255.70.252:8545
ROOT_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d  # Anvil Account 1（L2 有钱）
ROOT_ADDR=0x70997970C51812dc3A010C7d01b50e0d17dc79C8                       # = address(ROOT_PK)
SESSION_PK=0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a  # Anvil Account 2
SESSION_KEY=0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC                     # = address(SESSION_PK)
BYSTANDER=0x90F79bf6EB2c4f870365E785982E1f101E93b906
TEST_TOKEN=0x000000000000000000000000000000000000ABCD
KEYCHAIN_ADDR=0xAAAAAAAA00000000000000000000000000000000
SCI_AGENT_STATE_ADDR=0xAAAAAAAA00000000000000000000000000000001
```

### T1：基础健康（1 min）
```bash
cast chain-id --rpc-url $RPC                # 期望 42001
cast block-number --rpc-url $RPC            # 增长中
cast balance $ROOT_ADDR --rpc-url $RPC      # 非零
```

### T2：两个 precompile 都可达（1 min）
```bash
# Keychain getKey
cast call $KEYCHAIN_ADDR \
  'getKey(address,address)(uint8,uint64,bool,bool)' \
  $ROOT_ADDR $SESSION_KEY --rpc-url $RPC
# 未授权时期望 (0, 0, false, false)

# SciAgentState isTripped
cast call $SCI_AGENT_STATE_ADDR \
  'isTripped(address)(bool)' $SESSION_KEY --rpc-url $RPC
# 期望 false
```

### T3：普通 ETH 转账无回归（30 sec）
```bash
cast send --rpc-url $RPC --private-key $ROOT_PK $BYSTANDER --value 0.01ether
```
期望 status=1，gas 用 ~21000-21500（hook 走 fast-path 早退）。

### T4：authorize_key 端到端（2 min）
```bash
# Account 1 给 Account 2 授权（无限额、allow_any_calls=true）
cast send $KEYCHAIN_ADDR \
  'authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))' \
  $SESSION_KEY 0 '(18446744073709551615,false,[],true,[])' \
  --rpc-url $RPC --private-key $ROOT_PK

# 验证写入
cast call $KEYCHAIN_ADDR \
  'getKey(address,address)(uint8,uint64,bool,bool)' \
  $ROOT_ADDR $SESSION_KEY --rpc-url $RPC
# 期望 (0, 18446744073709551615, false, false)  -- expiry=MAX, enforce=false, revoked=false
```

### T5：SciAgentState access control（1 min）
```bash
# 用非 CB 地址试 trip —— 应该 revert with Unauthorized()
cast send $SCI_AGENT_STATE_ADDR 'tripKey(address)' $SESSION_KEY \
  --rpc-url $RPC --private-key $ROOT_PK
# 期望 revert，revert data 以 0x82b42900 开头（Unauthorized() 4-byte selector）
```

### T6：7702 不破坏 normal traffic（2 min）
```bash
RANDOM_DELEGATE=0x00000000000000000000000000000000000DEADBE

# Session_key 把自己 delegate 到一个随机地址（不是 SCI_AGENT_DELEGATOR_ADDRESS）
AUTH=$(cast wallet sign-auth $RANDOM_DELEGATE --private-key $SESSION_PK --rpc-url $RPC)
cast send --rpc-url $RPC --private-key $SESSION_PK --auth $AUTH \
  $SESSION_KEY 0x

# 之后从 session_key 普通转账 —— hook 看到 wrong delegate → pass through
cast send --rpc-url $RPC --private-key $SESSION_PK $BYSTANDER --value 0.001ether
```
期望 两笔 tx 都成功。

### T7（可选）：长跑稳定性（>1 hr，tmux 后台）
```bash
ssh ubuntu@54.255.70.252 'tmux new -d -s sci-soak "for i in {1..3600}; do \
  cast send --rpc-url http://localhost:8545 --private-key 0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d \
    0x90F79bf6EB2c4f870365E785982E1f101E93b906 --value 0.0001ether; \
  sleep 1; done"'
```
中途任意时刻 `docker logs base-client --tail 100 | grep -i error` 应该干净。

---

# 出错回滚（不丢链状态）

如果 Phase D 后链停了 / base-client 起不来：

```bash
# 让 sci-chain 切回 v0.8.0 base，重新 build 旧镜像
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/sci-chain && \
  git stash push -u -m "save SCI work" && \
  git checkout v0.8.0 && \
  export PATH=$HOME/.foundry/bin:/usr/local/go/bin:$HOME/.cargo/bin:$PATH && \
  just devnet build-image client dev'

# Force-recreate base-client（拉旧镜像，链恢复）
ssh ubuntu@54.255.70.252 'cd ~/sci-dev/base-v0.8 && \
  docker compose --env-file etc/docker/devnet-env \
    -f etc/docker/docker-compose.yml \
    up -d --no-build --no-deps --force-recreate base-client'
```

恢复后 sci-chain 的 SCI 改动还在 stash 里（`git stash list`），可以继续 debug。

---

# 未覆盖的（等 Heath 落地）

T8 完整 agent-tx 闭环：需要 `SCIAgentDelegator.sol` 部到 `0xCCCC...01`，且 root EOA 用 EIP-7702 delegate 到它。

那时候才能验证：
- 强 R1 真实场景：hook 通过 → body revert → quota 未扣
- 强 R1 反例：hook 通过 → body 成功 → quota 扣对
- batch 部分失败 → 整个 batch 回滚（hook 内 checkpoint_revert）
- CircuitBreaker `0xBBBB...03` → SciAgentState forward 真正生效

我们 14 个 hook_e2e 测试在本地 InMemoryDB 已经验证过这些路径，devnet 层面 Heath 落地后只需 smoke test。

---

# 当前状态摘要

- ✅ 本地 sci-precompiles + base-common-evm + sci-precompile-abi 全编译通过
- ✅ 本地 307 + 14 + 74 = 395 测试全过
- ✅ Commit `ef4914ea8` 已 push 到 `origin/feat/p0-1-keychain`
- ✅ 代码已 rsync 到 `ubuntu@54.255.70.252:~/sci-dev/sci-chain/`（ops patches 保留）
- ⏸️ Phase A（远端 cargo check）—— **明天从这里开始**
- ⏸️ Phase B（build :local 镜像）—— 等 A
- ⏸️ Phase D（hot-swap base-client）—— 等 B
- ⏸️ T1-T7 功能测试 —— 等 D
- ⛔ T8 完整 agent tx —— 阻塞于 Heath 的 SCIAgentDelegator.sol 部署
