---
title: "P0-2 合约集成测试指南 — 用 keychain 精灵测合约"
audience: "落 P0-2 Solidity 合约的同学（Heath）"
prerequisite_branch: "feat/p0-1-keychain（已 merge 或作为 base）"
date: "2026-05-21"
note: "本文档放在 gitignored 的 sci/docs/analysis/ 下，是给 Heath 的中文 onboarding 资料。"
---

# P0-2 合约集成测试指南

本文档说明：在 `feat/p0-1-keychain` 已经把 keychain 精灵实现完之后，如何把 P0-2
的 Solidity 合约（`AgentAccessKeyRegistry` / `AgentBudgetController` /
`AgentCircuitBreaker` / `SCIAgentDelegator` / IDA ERC-721 + ERC-6551 TBA）
落上去并跑通端到端测试。假设你在 `feat/p0-2-contracts` 分支上（CLAUDE.md
`## Branches` 里的 "S" 角色）。

---

## 1. `feat/p0-1-keychain` 已经给你准备好了什么

把这个分支 merge / pull 进你的工作流后，在跑 `:sci` Docker image 的 devnet 上，
下面这些地址的状态：

| 地址 | 组件 | `:sci` devnet 上的状态 |
|---|---|---|
| `0xAAAAAAAA00000000000000000000000000000000` | `AccountKeychain`（Rust precompile） | **已可用**。genesis 里有 `code:"0xef"`，state 写入持久化，`IAccountKeychain.sol` 所有 ABI 方法都路由到精灵 |
| `0xAAAAAAAA00000000000000000000000000000001` | `SciAgentState`（Rust precompile，CB flag 存储） | **已可用**。同上，且只允许 `0xBBBB...03` 通过 `tripKey` / `untripKey` 写入 |
| `0xBBBBBBBB00000000000000000000000000000001` | `AgentAccessKeyRegistry`（你的合约） | **空地址**，等你部署 |
| `0xBBBBBBBB00000000000000000000000000000002` | `AgentBudgetController`（你的合约） | **空地址**，等你部署 |
| `0xBBBBBBBB00000000000000000000000000000003` | `AgentCircuitBreaker`（你的合约） | **空地址**，等你部署 |
| `0xCCCCCCCC00000000000000000000000000000001` | `SCIAgentDelegator`（你的合约，EIP-7702 set-code 目标） | **空地址**，等你部署或预 alloc |

Rust 的 pre-execution hook（`SciHandler`）也是 live 的：`:sci` devnet 上每笔 tx
都会经过它。但 hook 对**任何 `tx.to` 不是 7702-delegated 到
`SCI_AGENT_DELEGATOR_ADDRESS` 的 tx** 是 no-op。所以普通交易、你的合约部署都
不会被影响。Hook 只在你接好 EIP-7702 delegation 指向 `SCIAgentDelegator` 后
才真正激活 keychain 检查。

keychain 的 ABI（Rust + alloy `sol!` 宏）在
`sci/crates/precompile-abi/src/precompiles/account_keychain.rs`，对应的
Solidity interface `IAccountKeychain.sol` 应该放在
`sci/contracts/src/interfaces/`（你的范围）。

---

## 2. 三条测试路径

### 路径 A — 直接打共享的远端 devnet（推荐用于早期迭代）

我们已经在一台共享服务器上跑了基于 `feat/p0-1-keychain` 的 devnet。你不用搭任何
本地环境，只要 foundry / forge 就能开干。

```bash
# 连接信息
export L2_RPC=http://54.255.70.252:8545
export CHAIN_ID=42001

# 测试账号（Anvil 助记词；L2 上每个都有 10000 ETH）
# Account 0 留给 P0-1 owner（我），你用 account 1 避免 nonce 冲突。
export DEPLOYER_PK=0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d
export DEPLOYER_ADDR=0x70997970C51812dc3A010C7d01b50e0d17dc79C8
```

部署 + 测试：

```bash
cd sci/contracts
forge create \
  --rpc-url $L2_RPC \
  --private-key $DEPLOYER_PK \
  src/agent/AgentAccessKeyRegistry.sol:AgentAccessKeyRegistry

# 从合约里调 keychain 精灵
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)((uint8,address,uint64,bool,bool))' \
  $DEPLOYER_ADDR \
  0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC \
  --rpc-url $L2_RPC
```

**适合**：验证**合约逻辑 + 跟 keychain 精灵的联动**。合约部署到 forge 给你的随机
地址即可，不要求固定的 `0xBBBB...` / `0xCCCC...` 预部署地址。

**注意事项**：
- 共享主机——nonce 要协调。Account 0 留给 P0-1，你用 account 1。
- 链状态是共用的、有持久化。如果你需要从干净状态开始，先跟我打招呼再请求
  `just devnet down`（会丢链状态）。

### 路径 B — 把你的合约固化到固定的预部署地址上（T8 必经之路）

有些测试需要你的合约就在 `0xBBBB...01/02/03` 和 `0xCCCC...01` 这几个固定地址，
这就跟 keychain 精灵当初拿 `code:"0xef"` 那样，需要写进 genesis allocs。流程：

1. **你**：用 `forge inspect` 拿到每个合约的 **`deployedBytecode`（runtime code，
   不是 init bytecode）**：

   ```bash
   forge inspect AgentAccessKeyRegistry deployedBytecode
   forge inspect AgentBudgetController  deployedBytecode
   forge inspect AgentCircuitBreaker    deployedBytecode
   forge inspect SCIAgentDelegator      deployedBytecode
   ```

2. **你**：如果合约 constructor 会写状态（比如 immutable `owner`、初始 admin 等），
   把那些**初始 storage slot 的值**也列清楚。genesis alloc 的格式是：

   ```json
   {
     "0xbbbbbbbb00000000000000000000000000000001": {
       "nonce": "0x0",
       "balance": "0x0",
       "code": "0x<deployedBytecode>",
       "storage": {
         "0x0000000000000000000000000000000000000000000000000000000000000000": "0x000000000000000000000000<admin-address>",
         "...": "..."
       }
     }
   }
   ```

   `forge inspect <Contract> storageLayout` 能帮你把字段映射到 slot。

3. **把 JSON 片段发给我**。我合并到 `sci/devnet/sci-allocs.json`，然后重启 devnet。
   bring-up 流程在 `sci/docs/feat-p0-1-keychain-branch-summary.md` §4 写了。

4. devnet 重启后，`cast code 0xBBBB...01` 会返回你的 `deployedBytecode`，合约就像
   被部署在那个固定地址一样可以调用。

**关于 `SCIAgentDelegator`（`0xCCCC...01`）两个选项**：

- **B1 — Genesis 预部署**（同上流程）。简单但是静态的：delegator 一改就要重启 devnet。
- **B2 — 部署到普通地址 + 运行时 EIP-7702 set-code**。把 `SCIAgentDelegator` 部署到
  随机地址，每个 root EOA 在测试时通过 EIP-7702 `setCode` authorization 指过去。
  更接近生产环境（真实的 agent UX），但测试编排稍复杂。pre-execution hook 会读
  `tx.to` 的 7702 header，比对 `delegated_address == SCI_AGENT_DELEGATOR_ADDRESS`
  —— 所以两个选项都能跑通，挑你测试方便的。

**适合**：T8 完整 agent-tx 端到端测试，以及任何需要断言"合约就在那个固定地址"的测试。

**协调成本**：合约的 ABI 或 storage layout 一改就要重新 genesis alloc + 重启 devnet
（链状态丢）。所以建议合约接口先冻结，再请求 Path B 部署。

### 路径 C — 自己跑本地 devnet

你需要大量合约迭代、不想跟我们共用一个远端、想随时 reset 链状态的话，在自己机器上
跑全套 stack：

```bash
git clone https://github.com/OPENSCI-Labs/sci-chain.git
cd sci-chain
git checkout feat/p0-1-keychain   # 包含 keychain precompile + sci/devnet/ 配置
# 完整流程见 sci/docs/feat-p0-1-keychain-branch-summary.md。
# 关键步骤：
#   1. 从 ~/sci-dev/base-v0.8/（pure base 克隆）build base-only release images
#   2. 从这个 repo build SCI release images，tag 成 :sci
#   3. just devnet down；起 L1 stack；只起 setup-l2
#   4. 应用 sci/devnet/apply-sci-allocs.sh 到 .devnet/l2/configs/genesis.json
#   5. 用新 genesis hash patch rollup.json + rollup-conductor.json
#   6. 用 sci compose override 起 base-client + base-builder
```

**适合**：合约侧高频迭代，不想跟其他人协调。`just devnet down` 随便用。

**门槛**：docker buildx + Rust 1.93.1 + foundry；首次 cold build 大概 30 分钟。

---

## 3. 建议时间线

| 阶段 | 用哪条路径 | 你做什么 |
|---|---|---|
| 现在 → 合约 API 稳定 | **Path A** | 在 live keychain 精灵上验证合约逻辑，无地址约束，自由迭代 |
| 合约 API 冻结 | **Path B** | 给每个预部署合约准备 `deployedBytecode` + 初始 storage。我合并进 `sci/devnet/sci-allocs.json`，重启 devnet。T8 解锁 |
| 准备 merge 到 `main` 之前 | **PR 集成** | 把 `feat/p0-2-contracts` rebase 到 `feat/p0-1-keychain` 上（或者把 p0-1 merge 进你的分支）。两边一起出 PR |

---

## 4. Pre-execution Hook 对你的 `SCIAgentDelegator` 的硬性要求

为了让 T8 完整 agent-tx 闭环跑通，Rust 端的 hook 会按下面这个 ABI 解码外层 tx 的
calldata：

```solidity
function execute(Call[] calldata calls) external;

struct Call {
    address target;
    uint256 value;
    bytes   data;
}
```

ABI 定义在 `sci/crates/precompile-abi/src/predeploys/sci_agent_delegator.rs`。你的
Solidity `SCIAgentDelegator.execute(Call[])` 签名要跟它**逐字 bit-for-bit 一致** ——
hook 的 `decode_execute_batch` 一旦 ABI 对不上，会静默 fallback 成 single-call 模式，
batch tx 测试就会失败。

另外 `SCIAgentDelegator` 内部必须强制：

```solidity
require(getTransactionKey() != address(0), "no session key");
```

其中 `getTransactionKey()` 调 keychain 精灵的 `getTransactionKey()` 方法。
pre-execution hook 在判定一笔 tx 是合规的 agent tx 后（且仅在那时），才把 keychain
的 transient slot `transaction_key` 写成 session key。所以这个 require 是 session-key
authorization 真正起作用的支点——没有它，任何人都能直接调 `execute(...)` 绕过 hook。

参考：`CLAUDE.md` → "Pre-execution Hook Design (P0-1.7 / P0-1.8)" →
"Agent-tx identification (Q1)" 有完整的论证。

---

## 5. Pre-execution Hook 对 `AgentCircuitBreaker` 的预期

`AgentCircuitBreaker` 在 `0xBBBB...03`，是套在 Rust `SciAgentState` 精灵
（`0xAAAA...0001`）外面的 Solidity 门面。合约本身负责 admin 访问控制（只允许指定
admin 调用 trip / untrip）、emit event，然后 forward 到精灵：

```solidity
function tripKey(address sessionKey) external onlyAdmin {
    ISciAgentState(0xAAAAAAAA00000000000000000000000000000001).tripKey(sessionKey);
    emit AgentTripped(sessionKey, msg.sender, block.timestamp);
}
```

`SciAgentState.tripKey` 在 Rust 端会拒绝任何 `msg.sender != AGENT_CIRCUIT_BREAKER_ADDRESS`
（`0xBBBB...03`）的调用。这个检查在 precompile 里强制执行，所以你的合约门面是
**唯一能 trip key 的路径**。CLAUDE.md → "CircuitBreaker state location (Q3)" 有
为什么把 state 放 Rust、admin 放 Solidity 的论证。

---

## 6. 命令 cheat sheet

```bash
# devnet 健康检查
cast chain-id      --rpc-url $L2_RPC                   # 期望 42001
cast block-number  --rpc-url $L2_RPC                   # 应该在增长
cast code 0xAAAAAAAA00000000000000000000000000000000 --rpc-url $L2_RPC   # 期望 0xef

# 读 keychain
cast call 0xAAAAAAAA00000000000000000000000000000000 \
  'getKey(address,address)((uint8,address,uint64,bool,bool))' \
  $ROOT_ADDR $SESSION_KEY --rpc-url $L2_RPC

# 授权一个 session key（写，由 root EOA 签）
cast send 0xAAAAAAAA00000000000000000000000000000000 \
  'authorizeKey(address,uint8,(uint64,bool,(address,uint256,uint64)[],bool,(address,(bytes4,address[])[])[]))' \
  $SESSION_KEY 0 '(18446744073709551615,false,[],true,[])' \
  --rpc-url $L2_RPC --private-key $ROOT_PK

# 查 trip 状态
cast call 0xAAAAAAAA00000000000000000000000000000001 \
  'isTripped(address)(bool)' $SESSION_KEY --rpc-url $L2_RPC

# 签发 + 广播一个 EIP-7702 set-code authorization
AUTH=$(cast wallet sign-auth $SCI_AGENT_DELEGATOR_ADDR \
  --private-key $ROOT_PK --rpc-url $L2_RPC)
cast send --rpc-url $L2_RPC --private-key $ROOT_PK --auth $AUTH \
  $ROOT_ADDR 0x   # payload 任意——重点是 auth
```

---

## 7. 共享 devnet 的测试账号分配

为了避免在共享远端 devnet 上互相 nonce 冲突：

| Account | 地址 | 谁用 | 私钥 |
|---|---|---|---|
| 0 | `0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266` | P0-1 owner（keychain 侧，我） | `0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80` |
| 1 | `0x70997970C51812dc3A010C7d01b50e0d17dc79C8` | **P0-2（你）** | `0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d` |
| 2 | `0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC` | 测试里的 session-key 角色 | `0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a` |
| 3 | `0x90F79bf6EB2c4f870365E785982E1f101E93b906` | transfer 测试里的 bystander / 收款方 | （一般不需要） |

完整助记词：`test test test test test test test test test test test junk`。

---

## 8. 走 Path B 时，alloc fragment 怎么准备

一个完整可复现的提交看起来是这样：

```json
{
  "0xbbbbbbbb00000000000000000000000000000001": {
    "nonce": "0x0",
    "balance": "0x0",
    "code": "0x...<完整 deployedBytecode>",
    "storage": {
      "0x0000000000000000000000000000000000000000000000000000000000000000": "0x..."
    }
  },
  "0xbbbbbbbb00000000000000000000000000000002": {
    "...": "..."
  },
  "0xbbbbbbbb00000000000000000000000000000003": { ... },
  "0xcccccccc00000000000000000000000000000001": { ... }
}
```

发给我之前的验证步骤：

1. `forge build` 通过、本地 anvil 把这些合约 runtime 部署后跑过测试（Path A 路径）
2. `forge inspect <Contract> deployedBytecode` 输出稳定（可复现）
3. `forge inspect <Contract> storageLayout` 列出来，并且 constructor 写的字段都
   翻译成了显式的 `storage` 条目
4. JSON valid（`jq . your-fragment.json` 通过）

把 fragment 发我的时候，附上一行说明：自上次提交以来你改了什么。这样我知道是否
需要重启 devnet。

---

## 9. 参考资料

- 分支总览：[`sci/docs/feat-p0-1-keychain-branch-summary.md`](../feat-p0-1-keychain-branch-summary.md)
- 架构 + 规范：[`/CLAUDE.md`](../../../CLAUDE.md)
- keychain ABI（Rust）：`sci/crates/precompile-abi/src/precompiles/account_keychain.rs`
- SciAgentState ABI（Rust）：`sci/crates/precompile-abi/src/precompiles/sci_agent_state.rs`
- SCIAgentDelegator ABI（Rust）：`sci/crates/precompile-abi/src/predeploys/sci_agent_delegator.rs`
- Hook 设计：`CLAUDE.md` → "Pre-execution Hook Design (P0-1.7 / P0-1.8)"

---

## 10. 有疑问 / 需要协调

如果上面任何地方不清楚，或者你的合约设计需要 keychain 侧配合改（比如加个 precompile
方法、ABI 调一下），来跟我说一声。`sci/contracts/`（你的）和 `sci/crates/precompiles/`
（我的）放在同一个 repo 的 `sci/` 下，就是为了让两边能通过同时改这两块的 PR 协同
演进——别客气，需要就提。
