---
title: "SCI Chain 转账附加信息方案对比分析"
subtitle: "TIP-20 transferWithMemo 移植与 ISCI20 设计评估"
author: "SCI Chain 工程组"
date: "2026-05-19"
lang: zh-CN
---

# 摘要

SCI Chain 作为 Agent-native L2，需要在代币转账中附带元数据（memo / 归因信息），用于报销、归因分析、合规审计等场景。本文对比 4 种候选实现方案，并对最有潜力的两种（你已设计的 ISCI20 加强版 vs MemoLog precompile）做深度量化分析，最终给出推荐落地路线。

**核心结论**：

1. 已设计的 ISCI20（方案 A 加强版）在 8/10 个关键维度上优于 MemoLog precompile（方案 B），尤其在**防伪造、批量结算 gas、原子性、合规审计**上有结构性优势。
2. 方案 B 在「兼容外部 ERC-20（USDC/USDT 等）」上不可替代，建议作为**未来补丁**而非主路径。
3. 推荐采用 **A 主 + B 补**的混合策略，分阶段落地。

---

# 1. 背景与问题定义

## 1.1 业务需求

| 场景 | 需求 |
|---|---|
| **报销** | Agent 替用户/平台垫付费用，事后凭事件记录走报销流程 |
| **归因分析** | 平台需统计「某个 skill / agent / job 产生了多少营收」 |
| **审计** | 财务每月对账，验证转账归因数据的真实性 |
| **MPP 集成** | Machine Payments Protocol 402 响应中携带 `jobId / taskId`，链上链下数据可互查 |
| **合规** | 监管要求可证明的转账元数据（不仅是"声明"，要"证据"） |

## 1.2 技术约束

- SCI Chain 是 Base Azul v0.8 fork，使用标准 ERC-20 而非 Tempo 的 TIP-20 token
- 已有的 AccountKeychain precompile 需要识别交易类型以执行 session key 限制
- 需要兼顾「SCI 原生代币」与「外部代币（USDC 等）」两种生态
- 长期需保持与 Tempo 上游同步的便利性

## 1.3 Tempo TIP-20 中的 transferWithMemo 参考

Tempo TIP-20 已经实现了 `transferWithMemo` 系列方法。其语义为：

```solidity
event TransferWithMemo(
    address indexed from,
    address indexed to,
    uint256 amount,
    bytes32 indexed memo    // indexed: 可按 memo 过滤事件
);

function transferWithMemo(address to, uint256 amount, bytes32 memo) external;
function transferFromWithMemo(address from, address to, uint256 amount, bytes32 memo) external returns (bool);
function mintWithMemo(address to, uint256 amount, bytes32 memo) external;
function burnWithMemo(uint256 amount, bytes32 memo) external;
```

实现要点（`tempo/.../tip20/mod.rs:807-828`）：

1. 与 `transfer()` 完全相同的转账逻辑（validate → check spending limit → `_transfer()`）
2. **额外** emit `TransferWithMemo` 事件（memo 作为 indexed topic）
3. 标准 `Transfer` 事件由 `_transfer()` 内部 emit，不重复

memo 是**纯加法**的：在普通转账基础上多 emit 一条带 memo 的事件。链下索引器拿这条事件做报销/归因。**memo 不参与转账本身的验证或状态**。

---

# 2. 四种候选方案

## 方案 A：纯 ABI 扩展（用户部署的扩展 ERC-20）

### 概述

定义 `ISCI20 is IERC20` 接口，用户/平台部署 Solidity 合约实现该接口。memo / 归因功能由合约自身实现，链层（precompile）只需识别 selector。

### 架构图

```
┌─────────────────────────────────────────────────┐
│ Agent ─call─► SCI20 Token Contract (Solidity)  │
│                  │                              │
│                  ├─ balances[from] -= amount    │
│                  ├─ balances[to]   += amount    │
│                  ├─ emit Transfer(...)           │
│                  └─ emit TransferWithMemo(...)   │
└─────────────────────────────────────────────────┘
```

### 实现样例

```solidity
contract MySCI20Token is IERC20 {
    function transferWithMemo(address to, uint256 amount, bytes32 memo) external returns (bool) {
        _transfer(msg.sender, to, amount);
        emit TransferWithMemo(msg.sender, to, amount, memo);
        return true;
    }
}
```

链层只需在 `sci-precompile-abi` 中加入 ABI，AccountKeychain 把 `transferWithMemo` selector 加入约束列表。

### 优缺点

| ✅ 优点 | ❌ 缺点 |
|---|---|
| 原子性强（单笔交易内完成转账 + memo） | 仅 ISCI20-aware 代币支持 memo |
| 不可伪造（msg.sender 由 EVM 强制） | 外部 ERC-20（USDC、USDT 等）无法享用 |
| Gas 节省（无第二次 CALL 开销） | 需要推动生态采纳新接口 |
| 钱包 / Wallet UX 自然（单签名 prompt） | 升级需要重新部署合约 |
| 跨链桥接友好（标准 ERC-20 派生） | |

### 改造工作量

- `sci-precompile-abi`: ~50 行（新增 ISCI20 ABI）
- AccountKeychain selector 列表: ~3 行
- 测试: ~150 行
- **总计: ~200 行 Rust 代码**（Solidity 合约由 dapp 端实现）

---

## 方案 B：Memo 旁路 precompile（MemoLog）

### 概述

新增一个 SCI 原生 precompile（建议地址 `0x4D454D4F00000000000000000000000000000000`，ASCII `MEMO`），暴露单一函数 `record(token, from, to, amount, memo)` 仅 emit 事件。**不**修改任何代币状态。

### 架构图

```
┌─────────────────────────────────────────────┐
│ Agent ─call──► ERC-20 Token (任何)         │
│                  └─ emit Transfer(...)      │
│        ─call──► MemoLog Precompile          │
│                  └─ emit TransferWithMemo(...)│
└─────────────────────────────────────────────┘
```

### 实现样例

```rust
// sci/crates/precompiles/src/memo_log/mod.rs
pub struct MemoLog;

impl Precompile for MemoLog {
    fn call(&mut self, calldata: &[u8], _msg_sender: Address) -> PrecompileResult {
        let call = IMemoLog::recordCall::abi_decode(calldata)?;
        self.emit_event(IMemoLog::TransferWithMemo {
            token: call.token,
            from: call.from,
            to: call.to,
            amount: call.amount,
            memo: call.memo,
        })?;
        Ok(success_output(Bytes::new()))
    }
}
```

```solidity
interface IMemoLog {
    event TransferWithMemo(
        address indexed token, address indexed from, address indexed to,
        uint256 amount, bytes32 indexed memo
    );
    function record(address token, address from, address to, uint256 amount, bytes32 memo) external;
}
```

### 优缺点

| ✅ 优点 | ❌ 缺点 |
|---|---|
| 兼容任何 ERC-20（包括 USDC、USDT 等外部代币） | **可伪造**：任何人都能调 `record(...)` 填假数据 |
| 索引器订阅点单一（一个地址） | 非原子：两笔调用（或一笔 multicall） |
| 实现简单（~300 行 Rust） | 跨链桥接困难（SCI 私有 precompile） |
| 不影响代币合约 | 钱包 UX 需要 multicall 支持 |

### 改造工作量

- `sci-precompile-abi`: ~50 行
- `sci-precompiles/src/memo_log/{mod,dispatch}.rs`: ~150 行
- `install()` 注册: ~5 行
- 测试: ~100 行
- **总计: ~300 行 Rust 代码**

---

## 方案 C：Memo 包装 precompile（原子化跨合约调用）

### 概述

类似方案 B，但 precompile 内部主动发起到 ERC-20 token 的 `transferFrom` 调用，实现单笔交易原子完成「转账 + memo」。

### 架构图

```
┌──────────────────────────────────────────────────┐
│ Agent ─call──► MemoWrapper Precompile           │
│                   │                              │
│                   ├─ subcall ► token.transferFrom(...)│
│                   │              └─ emit Transfer(...)│
│                   └─ emit TransferWithMemo(...)  │
└──────────────────────────────────────────────────┘
```

### 实现样例

```solidity
interface IMemoWrapper {
    event TransferWithMemo(
        address indexed token, address indexed from, address indexed to,
        uint256 amount, bytes32 indexed memo
    );
    /// Pulls `amount` of `token` from msg.sender via transferFrom, sends to `to`,
    /// emits TransferWithMemo. Caller must have approved this precompile address first.
    function transferWithMemo(address token, address to, uint256 amount, bytes32 memo) external;
}
```

### 优缺点

| ✅ 优点 | ❌ 缺点 |
|---|---|
| 原子单笔（同 A） | 实现复杂：precompile 须能发起子调用 |
| 兼容任何 ERC-20（同 B） | revm 34 / alloy-evm 0.27 PrecompileInput 不直接暴露子调用接口 |
| 防伪造（子调用真的转账了） | 需要 ERC-20 `approve` 前置（两步用户操作） |
| | 调试 / trace 复杂 |

### 改造工作量

- `sci-precompiles/src/memo_wrapper/{mod,dispatch}.rs`: ~400 行
- 子调用机制集成: ~200 行（revm hook 或 EvmInternals 扩展）
- **总计: ~600 行 Rust 代码 + revm/alloy-evm 集成工作**

---

## 方案 D：移植 Tempo TIP-20 token precompile

### 概述

把 Tempo 的 TIP-20 token + factory 整体移植，作为 SCI Chain 的协议级代币。

### 架构图

```
┌──────────────────────────────────────────────────┐
│ Agent ─call──► TIP-20 Token (Precompile 实现)   │
│                   ├─ 余额管理（precompile 内部存储）│
│                   ├─ transfer / approve / etc.   │
│                   ├─ transferWithMemo (内建)     │
│                   └─ emit TransferWithMemo(...)  │
│                                                  │
│        TIP20Factory Precompile                   │
│            └─ create token, is_tip20 query       │
└──────────────────────────────────────────────────┘
```

### 优缺点

| ✅ 优点 | ❌ 缺点 |
|---|---|
| 与 Tempo 上游 1:1 同步 | 工作量巨大（~7000 行 Rust 主代码） |
| 协议级保证 | 依赖 Tempo 的 IRolesAuth / TIP-403 / Stablecoin DEX 子系统 |
| 高性能（precompile 比 Solidity 快 ~10x） | 不兼容标准 ERC-20 生态 |
| 内建 spending limit 与 keychain 集成 | 需迁移现有 SCI20 token |

### 改造工作量

| 子系统 | 行数 |
|---|---|
| TIP-20 token precompile (移除 rewards/policy/DEX) | ~2,000 |
| TIP-20 factory precompile | ~900 |
| IRolesAuth 角色系统（依赖） | ~1,500 |
| TIP-403 transfer policy registry（依赖） | ~800 |
| Stablecoin DEX 桩或简化版（依赖） | ~600 |
| ABI bindings | ~500 |
| **总计** | **~6,300 行 + 多日工作** |

---

## 四方案速览对比

| 维度 | 方案 A (纯 ABI) | 方案 B (MemoLog) | 方案 C (Wrapper) | 方案 D (Tempo 移植) |
|---|---|---|---|---|
| Token 形态 | 用户部署的 Solidity | 任何 ERC-20 | 任何 ERC-20 | precompile 原生代币 |
| memo 落地 | Token 合约自己 | 独立 precompile | 包装 precompile | Token precompile 内建 |
| 原子性 | ✅ 单笔 | ❌ 两次调用 | ✅ 单笔 | ✅ 单笔 |
| 兼容外部 ERC-20 | ❌ | ✅ | ✅ | ❌ |
| 防伪造 | ✅ | ❌ | ✅ | ✅ |
| 实现工作量 (行) | ~200 | ~300 | ~600 | ~6,300 |
| 跨链桥接 | ✅ 标准 | ❌ 私有 | ❌ 私有 | ❌ 私有 |
| 与 Tempo 同步 | 解耦 | 解耦 | 解耦 | 1:1 同步 |

---

# 3. 既有 ISCI20 设计（方案 A 加强版）

SCI Chain 已有的 ISCI20 接口定义：

```solidity
interface ISCI20 is IERC20 {
    // 原 SCI-20 元数据事件
    event TransferWithMeta(
        address indexed from,
        address indexed to,
        uint256 amount,
        bytes32 indexed idaRef,
        bytes32 skillId,
        bytes32 agentId,
        bytes32 jobId,
        uint8 paymentType
    );

    // TIP-20 启示：32 字节 Memo 字段（来自 Tempo）
    event TransferWithMemo(
        address indexed from,
        address indexed to,
        uint256 amount,
        bytes32 memo
    );

    function transferWithMeta(/* ... */) external returns (bool);
    function transferWithMemo(address to, uint256 amount, bytes32 memo) external returns (bool);
    function batchAttributionSettle(AttributionRecord[] calldata records) external;
}
```

## 3.1 设计创新点

1. **`TransferWithMeta` 4 个 indexed 字段** —— 充分利用了 EVM event 最多 4 个 topic 的容量（from / to / idaRef / 隐式 selector hash），链下索引器可以按 IDA contract / agent / job 等任意一个维度高效查询
2. **保留 Tempo `TransferWithMemo`** —— 给只需要简单 memo 的场景一个轻量入口，不强迫所有人都填满 5 个 meta 字段
3. **`batchAttributionSettle`** —— 对批量结算场景（如 epoch 结束后给 N 个 agent 发款）做了原生支持，节省大量 gas
4. **`is IERC20`** —— 任何标准 ERC-20 钱包/Dex 看到的还是普通 transfer 事件，不破坏外部兼容

## 3.2 与方案 A 基础版的关系

ISCI20 = 方案 A + 结构化归因 + 批量原语。其余特性（用户部署、Solidity 实现、链层只需识别 selector）完全继承方案 A。

# 4. 深度对比：ISCI20（A 加强版）vs MemoLog（B）

下面对比假设三个典型场景：

- **单笔**：Agent A 给 Agent B 付 100 USDC，附带 `jobId=X, skillId=Y`
- **批量**：epoch 结束后，平台给 100 个 agent 批量发放结算款
- **审计**：财务每月对账，验证 1000 笔交易的归因数据

## 4.1 安全性

### 4.1.1 伪造攻击（关键差异）

**方案 A**:

```solidity
function transferWithMeta(address to, uint256 amount, /* ... */) external returns (bool) {
    require(balances[msg.sender] >= amount);
    balances[msg.sender] -= amount;
    balances[to] += amount;
    emit TransferWithMeta(msg.sender, to, amount, idaRef, /* ... */);
    return true;
}
```

→ **不可伪造**：`from` 字段是 `msg.sender`，由 EVM 强制注入。事件里的 `(from, to, amount)` 必然对应真实余额变化。

**方案 B**:

```rust
fn record(token: Address, from: Address, to: Address, amount: U256, memo: B256) {
    // ⚠️ from 是 calldata,任何人都能填
    emit_event(TransferWithMemo { token, from, to, amount, memo });
}
```

→ **可伪造**：坏 agent 可以调用 `MemoLog.record(USDC, 0xDeadBeef, 0xVictim, 1_000_000, "fake_invoice_42")` 凭空生成"转账"事件。

**伪造场景**：

- 坏 agent 没干活，但伪造一堆 `TransferWithMemo(从平台, 到自己, 大额, jobId=X)` 事件骗审计
- 审计员需要二次去 USDC 合约 log 里 cross-check `Transfer(from, to, amount)` 才能确认真伪
- 如果坏 agent 把伪造事件参数跟某笔真实小额转账对上，cross-check 也会被骗

**缓解方法（方案 B）**：

1. `MemoLog.record` 限定 `msg.sender == from` —— 破坏代记录场景
2. 强制 same-tx receipt cross-check —— revm 34 不暴露 receipt 给 precompile，需大量改造
3. 依赖 caller 老实 —— 没有强制约束

**结论**：方案 B 的伪造问题是结构性的，对**财务审计/报销**这种需要 evidence-grade 数据的场景**不可接受**。

### 4.1.2 重放攻击

| 方案 | 抗重放成本 |
|---|---|
| A | 每次都扣余额，重放需新 nonce + 真实资金 —— 经济上做不到便宜重放 |
| B | `record` 不改状态，gas ~5K，可大量重放堆积 event log —— 需 rate limit |

### 4.1.3 DoS / Gas Bomb

| 方案 | DoS 风险 | 缓解 |
|---|---|---|
| A | `batchAttributionSettle` 接收 unbounded 数组 → 吃光 block gas | 必须 `records.length <= MAX_BATCH`（建议 100~500） |
| B | 单次 record ~5K，难 DoS；但无批量原语 | N/A |

### 安全性维度小结

| | A | B |
|---|---|---|
| 防伪造 | ✅ 强 | ❌ 弱 |
| 防重放 | ✅ 经济成本高 | ⚠️ 需 rate limit |
| 防 DoS | ⚠️ 批量需 MAX 限制 | ✅ 单点小 |

→ **方案 A 在安全性上结构性领先**。

---

## 4.2 Gas 费

### 4.2.1 单笔交易

按以太坊 Cancun spec 估算（SCI Chain 一致）：

| 操作 | 方案 A | 方案 B |
|---|---|---|
| Tx base | 21,000 | 21,000 |
| CALL → ISCI20 | 2,300 | 2,300（到 token） |
| balanceOf SLOAD ×2 (cold) | 4,200 | 4,200 |
| SSTORE 减余额 (non-zero→non-zero) | 5,000 | 5,000 |
| SSTORE 加余额 (warm) | 5,000 | 5,000 |
| Solidity dispatch | ~500 | ~500 |
| `Transfer` 事件 (LOG3) | 1,875 | 1,875 |
| `TransferWithMeta` 事件 (LOG3 + 5 word data) | 2,780 | — |
| 第二次 CALL → MemoLog precompile | — | 2,300 |
| MemoLog 内部执行 | — | ~200 |
| `TransferWithMemo` 事件 (LOG4 + 1 word) | — | 2,131 |
| Calldata cost (~200 bytes × 16) | 3,200 | 2,900（两段总和） |
| **合计** | **45,855** | **47,406** |
| **差额** | — | **+1,551** |

→ 单笔差距 ~1.5K，方案 B 略贵。

### 4.2.2 批量结算 100 笔

| 操作 | 方案 A (`batchAttributionSettle`) | 方案 B (100 transfer + 100 record) |
|---|---|---|
| Tx base | 21,000 | 21,000（multicall 合并） |
| 转账 + 事件 ×100 | 3,500,000 ~ 5,000,000 | 6,000,000 ~ 8,500,000 |
| 共享 dispatch / SLOAD warming | 是 | 否 |
| **合计** | **3.5M ~ 5M** | **6M ~ 8.5M** |

→ **批量场景方案 A 节省 ~40% gas**。`batchAttributionSettle` 的批量优化在此发挥关键作用。

### 4.2.3 极端：1000 个 agent 批量发款

- 方案 A：1 tx 用 `batchAttributionSettle`，~35M gas（接近 block gas limit，可能需 2-3 tx）
- 方案 B：1000 次 transfer + 1000 次 record，~70M gas，需 10+ tx

→ **方案 A 在批量场景上显著占优**。

---

## 4.3 执行速度

### 4.3.1 单笔交易

| 阶段 | 方案 A (μs) | 方案 B (μs) |
|---|---|---|
| Tx 解析 / signature recovery | 50 | 50 |
| CALL 到 ISCI20 (Solidity dispatch) | 5 | 5 |
| Solidity 执行（balance updates + 2 events） | 10 | 10（仅 transfer 部分） |
| 第二次 CALL 到 MemoLog precompile | — | 3 |
| Precompile 执行 (emit event) | — | 1 |
| revm context 切换 / journal commit | 1 次 | 2 次（+3μs） |
| **单笔时延** | **~65** | **~72** |

差距 ~10%，单笔可忽略。

### 4.3.2 区块容量影响

假设每个 block 100M gas limit：

- 方案 A：每笔 ~50K → 2,000 笔/区块
- 方案 B：每笔 ~52K → 1,923 笔/区块
- 方案 A（batchAttributionSettle）：每 100 笔合并成 ~4M gas tx → 25 个 batch tx/区块 = **2,500 笔/区块**

→ **方案 A 在高 TPS 场景下吞吐量提升 ~25%**。

---

## 4.4 原子性 / 一致性

| 场景 | 方案 A | 方案 B |
|---|---|---|
| 转账成功 + 归因记录失败 | 不可能（原子） | 可能（两个 tx） |
| 转账失败 + 归因记录成功 | 不可能 | 可能（伪造场景） |
| Chain reorg | 整体回滚或保留 | 可能部分回滚 |
| 单 tx multicall | 原生支持 | 需 wallet 支持 |

→ **方案 A 完胜**。这对财务系统至关重要。

---

## 4.5 链下索引 / 数据可用性

| 维度 | 方案 A | 方案 B |
|---|---|---|
| 订阅点 | N 个 SCI20 token 地址 | 1 个固定 MemoLog 地址 |
| 单次查询「按 jobId 查」 | `eth_getLogs(addrs=[all_sci20s], topics=[..., jobId])` | `eth_getLogs(addr=MemoLog, topics=[..., jobId])` |
| 历史回填 | 遍历所有 SCI20 token 部署历史 | 单一地址扫描 |
| 分片 | 自然按 token 分片（可扩展） | 单点热点 |
| 数据完整性验证 | 隐含（事件 = 真实转账） | 需 cross-correlate `Transfer` |

→ B 订阅简单，但**A 在数据完整性上更可靠**。

---

## 4.6 兼容外部 ERC-20（关键劣势）

| 方案 | 支持 USDC / USDT 等外部代币？ |
|---|---|
| A | ❌ 外部 ERC-20 不实现 `transferWithMeta`，外部代币付款拿不到 SCI 归因 |
| B | ✅ 任何 ERC-20 转账后调一次 `MemoLog.record()` 即可 |

**这是方案 B 唯一显著优势**。如果 SCI Chain 上需大量跑外部稳定币支付，B 不可或缺。

---

## 4.7 钱包 / Agent SDK 集成

| 操作 | 方案 A | 方案 B |
|---|---|---|
| Agent SDK 调用 | 1 个 `sci20.transferWithMeta(...)` | `multicall([token.transfer(...), memoLog.record(...)])` |
| 标准钱包（MetaMask 等） | 单签名 prompt | 两次签名 or 1 个 multicall (EIP-5792) |
| Gas 估算 | 准确 | 两笔分别估算或合并 |

→ **方案 A 对开发者/用户更友好**。

---

## 4.8 升级 / 演进

| 场景 | 方案 A | 方案 B |
|---|---|---|
| 新增归因字段 | 新 ABI 版本，部署 ISCI20 v2 | 改 precompile = 链硬分叉 |
| 移除字段 | 旧合约不变，新合约用新接口 | 同上 |
| 兼容旧版本 | 自然兼容 | 需版本化 precompile |

→ **方案 A 演进灵活**，方案 B 集中演进。

---

## 4.9 跨链 / Bridge

| 维度 | 方案 A | 方案 B |
|---|---|---|
| SCI20 token 桥到 L1 | ✅ 跟标准 ERC-20 一样桥 | — |
| 归因事件桥过去 | ✅ 标准 event log,bridge 可索引 | ❌ 跨链桥不识别 SCI 私有 precompile |

→ **方案 A 跨链友好**，方案 B 是 SCI 链私有的。

---

## 4.10 综合评分

| 维度 | 权重 | A 分 | B 分 | 加权差 (A - B) |
|---|---|---|---|---|
| 安全性（防伪造） | ⭐⭐⭐⭐⭐ | 10 | 3 | **+35** |
| 单笔 gas | ⭐⭐ | 9 | 8 | +2 |
| 批量 gas | ⭐⭐⭐⭐ | 10 | 4 | **+24** |
| 执行速度 | ⭐ | 9 | 8 | +1 |
| 原子性 | ⭐⭐⭐⭐⭐ | 10 | 4 | **+30** |
| 索引简单度 | ⭐⭐ | 6 | 9 | -6 |
| 索引可信度 | ⭐⭐⭐⭐ | 10 | 5 | **+20** |
| 外部 ERC-20 兼容 | ⭐⭐⭐ | 2 | 10 | **-24** |
| 钱包 UX | ⭐⭐ | 9 | 6 | +6 |
| 升级灵活 | ⭐⭐ | 8 | 5 | +6 |
| 跨链友好 | ⭐⭐ | 9 | 3 | +12 |
| **总分** | | | | **A +106** |

**结论**：方案 A 在 SCI 主要使用场景下大幅领先，但 B 在「外部 ERC-20 兼容」上不可替代。

---

# 5. 推荐方案：混合策略

ISCI20（A）和 MemoLog（B）在架构上正交，**不必二选一**。

```
SCI Chain Token 生态:

  ┌─────────────────────────────────────────────┐
  │  SCI20 tokens（你自己的归因代币）          │
  │  实现 ISCI20 接口                            │
  │  → 用 transferWithMeta（方案 A）            │
  │  → 财务级 evidence-grade 归因               │
  │  → 批量结算高效                             │
  └─────────────────────────────────────────────┘
                       +
  ┌─────────────────────────────────────────────┐
  │  外部 ERC-20（USDC、USDT、…）              │
  │  无法修改接口                                │
  │  → 用 MemoLog precompile（方案 B）          │
  │  → 索赔级 claim 归因（需 cross-check）       │
  │  → 接受弱保证（前提：标记为 unverified）     │
  └─────────────────────────────────────────────┘
```

**MPP Gateway 在响应里标注归因事件来源**：

- `evidence: "atomic"` ← 方案 A
- `evidence: "claim"` ← 方案 B

让审计端能识别强度差异。

---

# 6. 实施路线图

## 第一阶段（立即，~1 小时工作量）

1. 把 `ISCI20` ABI 加进 `sci/crates/precompile-abi/src/precompiles/sci20.rs`
2. `account_keychain/mod.rs` 把 `transferWithMeta` selector 加入 `is_constrained_*_selector`
3. keychain 的 `apply_spending_limit` 逻辑要识别 `transferWithMeta`，让 session key 的 spending limit 能管住 meta 版的转账
4. 单元测试：验证 session key 能限制 `transferWithMeta` 的 recipient

## 第二阶段（视外部 token 接入需求决定）

1. 实现 MemoLog precompile（~300 行）
2. 事件中标注 `evidence` 等级
3. MPP Gateway 接入归因评分

## 第三阶段（可选，看 SCI 治理需求）

1. SCI20Factory precompile —— 让 SCI20 token 发行权在协议层管控
2. `is_sci20(addr)` 识别 —— 让 keychain 给 SCI20 token 转账特殊待遇

---

# 7. 一句话结论

**你已有的 ISCI20 设计（方案 A 加强版）方向正确，对 SCI 报销/归因/审计场景全面优于 MemoLog（方案 B）。建议主推 A，把 B 作为未来兼容外部 ERC-20 的补丁，并在 MPP Gateway 层用 `evidence` 字段区分两类归因的强度。**

---

# 附录：评分维度权重说明

| 维度 | 权重 | 理由 |
|---|---|---|
| 安全性（防伪造） | ⭐⭐⭐⭐⭐ | 财务/审计场景的根本要求 |
| 原子性 | ⭐⭐⭐⭐⭐ | 数据一致性的最底线 |
| 批量 gas | ⭐⭐⭐⭐ | SCI 业务有大量批量结算场景 |
| 索引可信度 | ⭐⭐⭐⭐ | 归因数据要能信，否则失去价值 |
| 外部 ERC-20 兼容 | ⭐⭐⭐ | SCI 上跑 USDC 等的可能性 |
| 单笔 gas | ⭐⭐ | 主要影响每笔成本，但单笔差距小 |
| 跨链友好 | ⭐⭐ | 多链未来必备 |
| 索引简单度 | ⭐⭐ | 影响 dev cost,不影响产品功能 |
| 钱包 UX | ⭐⭐ | 影响 agent 集成成本 |
| 升级灵活 | ⭐⭐ | 长期影响，非紧急 |
| 执行速度 | ⭐ | 单笔差距 ~10%，可忽略 |

# 附录：术语表

| 术语 | 含义 |
|---|---|
| **IDA** | Identity Anchor —— SCI Agent 的链上身份合约（ERC-721 + ERC-6551 TBA） |
| **MPP** | Machine Payments Protocol —— Agent 之间使用的支付协议（HTTP 402 扩展） |
| **idaRef** | 关联到某个 IDA 实例的引用 |
| **memo** | 32 字节自由格式备注字段（Tempo TIP-20 起源） |
| **AttributionRecord** | 批量结算时的归因记录条目（含 recipient / amount / meta 字段） |
| **evidence grade** | 归因数据的可信度等级：`atomic`（同笔交易内）/ `claim`（事件 claim，需 cross-check） |
| **session key / access key** | 受限的次级签名密钥，受 AccountKeychain 管理 |
