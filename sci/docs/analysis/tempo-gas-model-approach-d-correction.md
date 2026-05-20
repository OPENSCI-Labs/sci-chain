---
title: "Tempo Gas 经济模型与 SCI Chain 方案 D 表述修正"
subtitle: "pathUSD 的本质 / 外部 ERC-20 在 TIP-20 链上的地位 / SCI 应否走 Tempo 路线"
author: "SCI Chain 工程组"
date: "2026-05-19"
lang: zh-CN
---

# 摘要

本文回应一个关键追问：**Tempo 的 gas 用稳定币——这个稳定币究竟是外部 ERC-20 还是 Tempo 原生 token？如果是 ERC-20，那么我之前对方案 D（移植 Tempo TIP-20 token precompile）"不兼容外部 ERC-20" 的描述是否有问题？**

通过 Tempo 源码核查，得到三个结论：

1. **pathUSD 是 Tempo 链原生的 TIP-20 token**（precompile-backed），地址前缀 `0x20C0`，不是外部 ERC-20。
2. Tempo 整个 gas 经济**完全建立在 TIP-20 之上**，外部 ERC-20 没有资格做 gas。
3. **我之前对方案 D 的描述确实不够精确**——准确说法应为「外部 ERC-20 是二等公民（能存在，但拿不到 TIP-20 协议级特性）」。本文给出修订表述与对 SCI Chain 的设计建议。

---

# 1. 事实核查：pathUSD 是 Tempo 链原生的 TIP-20

## 1.1 地址结构与命名空间

```rust
// tempo/crates/contracts/src/precompiles/mod.rs:29-30
pub const PATH_USD_ADDRESS: Address = address!("0x20C0000000000000000000000000000000000000");
pub const DEFAULT_FEE_TOKEN: Address = PATH_USD_ADDRESS;
```

地址 `0x20C0000000000000000000000000000000000000` 的前缀 **`0x20C0`** 就是 Tempo 的 TIP-20 地址前缀。

```rust
// tempo/crates/primitives/src/address.rs
pub fn is_tip20_prefix(addr: Address) -> bool { ... }
const TIP20_PREFIX: [u8; 12] = TIP20_TOKEN_PREFIX;  // 0x20C0...
```

## 1.2 自动 precompile 路由

任何带 `0x20C0` 前缀的地址，会被 Tempo 在 EVM 配置时**自动路由到 TIP-20 Token precompile dispatcher**：

```rust
// tempo/crates/precompiles/src/lib.rs:118
precompiles.set_precompile_lookup(move |address: &Address| {
    if address.is_tip20() {
        Some(TIP20Token::create_precompile(*address, &cfg))  // 协议级 precompile
    } else if *address == TIP20_FACTORY_ADDRESS {
        Some(TIP20Factory::create_precompile(&cfg))
    } else if *address == TIP403_REGISTRY_ADDRESS {
        ...
    }
});
```

## 1.3 Fee Manager 的硬假设

Tempo 的 fee manager precompile 在处理 gas 时**直接 cast token 地址为 TIP-20**：

```rust
// tempo/crates/precompiles/src/tip_fee_manager/mod.rs:201
let mut tip20_token = TIP20Token::from_address(fee_token)?;  // 必须能转成 TIP20

// 若用户付的 stable 不是 validator 接受的 token,
// 自动用 StablecoinDEX 做 fee swap
if fee_token != validator_token && !actual_spending.is_zero() {
    self.execute_fee_swap(fee_token, validator_token, actual_spending)?;
}
```

## 1.4 综合结论

| 维度 | 答案 |
|---|---|
| pathUSD 是外部 ERC-20 bridge 进来的吗？ | ❌ 不是 |
| pathUSD 是 user-deployed Solidity 合约吗？ | ❌ 不是 |
| pathUSD 是什么？ | ✅ 协议级 TIP-20 precompile token，链原生 |
| 外部 ERC-20 在 Tempo 上能做 gas 吗？ | ❌ 不能（fee manager 直接 reject） |
| 多个 TIP-20 stablecoin 之间能互换做 gas 吗？ | ✅ 通过 StablecoinDEX 自动 fee swap |

→ Tempo 的整个 gas 经济**完全建立在 TIP-20 协议级机制之上**。外部 ERC-20 没有资格做 gas。

---

# 2. 修正：方案 D 的表述不够精确

我之前在方案 D 的"缺点"里写：

> ❌ 不兼容标准 ERC-20 生态

这个表述**是误导的**。修正如下：

## 2.1 方案 D 实际并不是"拒绝外部 ERC-20"

外部 ERC-20（USDC、USDT 等）**仍然可以照常部署到 SCI Chain**，作为普通 EVM Solidity 合约：

| 操作 | 外部 ERC-20 在 Approach D 下能用吗 |
|---|---|
| 部署 ERC-20 合约 | ✅ 能（普通 Solidity 合约） |
| transfer / approve / balanceOf | ✅ 能 |
| Agents 持有、转账、调用 | ✅ 能 |
| 区块浏览器识别 | ✅ 能（标准 Transfer 事件） |
| 跟其他合约交互 | ✅ 能（标准 ERC-20 接口） |

## 2.2 外部 ERC-20 真正失去的是 TIP-20 协议级特性

| TIP-20 特性 | 外部 ERC-20 享有? |
|---|---|
| 作为 gas 支付 | ❌ Fee manager 不认 |
| Memo / 归因事件（`transferWithMemo`） | ❌ 接口不存在 |
| Fee 折扣 / 协议级 spending limit | ❌ Keychain 不识别为 TIP-20 |
| 进 SCI 原生 StablecoinDEX（若有） | ❌ 不是 TIP-20 |
| TIP20Factory.is_tip20() 识别 | ❌ 不是 |
| 协议级 transfer policy（TIP-403） | ❌ 不进入 policy 路径 |

→ 外部 ERC-20 在 Approach D 下变成"二等公民"：能转账、能存在，但拿不到协议级特性。

## 2.3 修订后的方案 D 描述

旧表述：

```
| ❌ 缺点 |
|---|
| 不兼容标准 ERC-20 生态        |
| 需迁移现有 SCI20 token        |
```

修订为：

```
| ❌ 缺点 |
|---|
| 外部 ERC-20 是「二等公民」: 能存在, 但无 TIP-20 协议级特性    |
| （无法做 gas、无 memo、无 fee 折扣、无 protocol-level spending limit） |
| 若 SCI 想让 USDC 等外部稳定币做 gas, 必须先 wrap 成 SCI 原生 TIP-20  |
| 需迁移现有 SCI20 token       |
| 依赖 Tempo 的 IRolesAuth / TIP-403 / Stablecoin DEX 子系统     |
| 工作量巨大（~7000 行 Rust 主代码） |
```

---

# 3. Tempo 为什么这样设计 gas 经济

Tempo 的 product 定位是**银行/企业级稳定币 L1**，做这些选择有特定原因：

| 设计点 | 理由 |
|---|---|
| Gas 用稳定币（不用 native ETH-like） | 企业用户接受不了 gas 价格暴跌暴涨（要预算可控） |
| Stablecoin 必须是 TIP-20 protocol-native | 监管/合规、KYC/AML 要在协议层强制（外部 ERC-20 做不到） |
| TIP-403 transfer policies | 黑名单、地区限制等合规规则在协议层 |
| StablecoinDEX 用作 fee swap | 用户用任何接受的 stable 付 gas，validator 收到指定 token |
| 没有 native gas token（无 ETH 角色） | 简化经济模型，所有价值都以 USD 衡量 |

→ Tempo 的整个 stack 是**为「稳定币原生」优化的**。外部 ERC-20 在这种设计里就是"边缘资产"，根本不是设计目标。

## 3.1 Tempo gas 流程图（端到端）

```
Agent 发起 tx (gas paid in pathUSD)
     │
     ▼
┌──────────────────────────────────────────────────────┐
│ revm 进入 TIP_FEE_MANAGER precompile (0xfeec...)    │
│   ├─ 检查 fee_token 必须是 TIP-20 (address.is_tip20)│
│   ├─ 若 fee_token != validator_token:                │
│   │    StablecoinDEX swap → validator_token         │
│   ├─ TIP20Token::from_address(validator_token)      │
│   └─ 扣减 sender balance, 增加 validator balance     │
└──────────────────────────────────────────────────────┘
     │
     ▼
正常执行 tx (call contracts / etc.)
```

整个流程**没有任何一步会接受外部 ERC-20 作为 fee token**。

---

# 4. SCI Chain 的设计抉择

这场分析暴露了一个**关键的产品决策**：SCI Chain 要不要走 Tempo 路线（native stablecoin-as-gas）？

## 4.1 三种取向

### 取向 A：跟 Tempo 一样，自发 stablecoin 当 gas

- SCI 发行自己的 stablecoin（如 `sciUSD`），作为 SCI Chain 的 gas 代币
- 实现为 TIP-20 precompile（走 Approach D）
- 外部 ERC-20 仍可流通，但只是「非 gas 资产」

| ✅ 优势 | ❌ 劣势 |
|---|---|
| gas 价格稳定 | 需要发行/储备/锚定机制（法币储备 or 算法稳定） |
| 合规可控（KYC/AML 在协议层） | 运营成本高（合规、法务、储备金管理） |
| 跟 Tempo 同步顺畅 | 需要 6,300+ 行 Rust 代码 + 多日工作 |
| 协议级 transfer policy（黑名单） | 跟 SCI 的「Agent-native」定位略有错位 |

### 取向 B：用外部 stablecoin 当 gas（USDC/USDT）

- SCI Chain 接受 USDC/USDT 作 gas（类似 Polygon 的 paymaster 模式 + EIP-7702）
- 不需要发 stablecoin
- 外部 ERC-20 是 first-class

| ✅ 优势 | ❌ 劣势 |
|---|---|
| 零发行成本 | 合规风险（Circle、Tether 可冻结资产） |
| 用户用熟悉的 stable | 需要 paymaster 基础设施 |
| 不破坏 SCI 的 Ethereum-native 兼容性 | gas 价格仍受外部 stablecoin 波动影响 |

### 取向 C：双轨制（推荐）

- 默认 gas 用 SCI 原生 native token（如 ETH-like，无稳定币特性）
- 通过 ERC-4337 paymaster 让 agents 选择用 USDC/USDT 付 gas（间接）
- SCI 自己**不**强制 stablecoin-as-gas

| ✅ 优势 | ❌ 劣势 |
|---|---|
| 最大兼容性 | gas 价格波动 |
| 可演进（未来加 stablecoin gas 是可选） | agent 经济性差（要付波动的 native token） |
| 不破坏 Ethereum-style 用户习惯 | 需要 paymaster 基础设施才能达到「稳定 gas」 |

## 4.2 推荐：取向 C 双轨制

根据 SCI 当前 product 定位（Agent-native L2，主推 MPP）：

- **SCI Chain 当作 EVM-compatible L2**，用类似 ETH 的 native gas + 4337 paymaster 让 agent 选 USDC/USDT 付 gas
- **不要走 Approach D 路径**做 SCI 原生 TIP-20 token system，理由：
  1. 工作量巨大（6300+ 行 + 监管/锚定机制）
  2. SCI 的 differentiation 是 **keychain + MPP + 归因**，**不是稳定币发行**
  3. 把发 stablecoin 留给生态合作伙伴（让 Circle 在 SCI 上部 USDC contract，或用桥引入）

## 4.3 这个抉择对之前推荐的影响

之前我推荐了「方案 A（ISCI20）主推 + 方案 B（MemoLog）补位」。这个推荐**在取向 C 下依然成立**：

```
SCI Chain 代币生态:

┌────────────────────────────────────────────────────┐
│ Native gas token（ETH-like, 由 SCI Chain 协议产出）│
│   └─ 用于支付 gas                                    │
└────────────────────────────────────────────────────┘
                       +
┌────────────────────────────────────────────────────┐
│ SCI20 tokens（你自己的归因代币, ISCI20 实现）     │
│   ├─ ERC-20 合约 + transferWithMeta 扩展            │
│   └─ 用于 SCI 内部归因、报销、agent-agent 支付      │
└────────────────────────────────────────────────────┘
                       +
┌────────────────────────────────────────────────────┐
│ 外部 ERC-20（USDC、USDT、由桥引入或社区部署）     │
│   ├─ 标准 ERC-20 合约                               │
│   ├─ 用 MemoLog precompile 加归因（claim-grade）   │
│   └─ 经 paymaster 间接可做 gas                      │
└────────────────────────────────────────────────────┘
```

**不需要 TIP-20 token precompile**（不走 Approach D）。

---

# 5. 总结

## 5.1 三个核心更正

1. **pathUSD 是 Tempo 协议级 TIP-20 precompile token**，不是外部 ERC-20。
2. **Tempo 的 gas 经济完全 TIP-20 化**，外部 ERC-20 没有资格做 gas。
3. **我之前的方案 D 描述不精确**——外部 ERC-20 在 Approach D 下能存在，但失去 TIP-20 协议级特性。

## 5.2 给 SCI Chain 的建议

- **不走 Approach D**——发行自己的 stablecoin 不是 SCI 的 differentiation
- **采用取向 C**——native gas token + 4337 paymaster 让 agents 间接用 USDC/USDT
- **代币生态分三层**：native gas / SCI20 归因代币 / 外部 ERC-20
- **方案 A（ISCI20） + 方案 B（MemoLog） 双管齐下**——这个推荐依然成立

## 5.3 长期演进的开放选项

如果未来 SCI 业务需要"稳定的 gas 价格"作为核心卖点（比如服务监管严格的金融机构），可以**追加一个简化版的 stablecoin-as-gas 机制**：

- **不**走完整 Tempo TIP-20 路径（避免 6300+ 行）
- **而是**实现一个 lightweight fee paymaster precompile，预存 USDC 替用户付 gas
- 这跟 Approach C（包装 precompile）的设计哲学一致

但**这是未来的事**，现在不做。

---

# 附录 A：Tempo Gas 设计深度参考

| 关键文件 | 行号 | 作用 |
|---|---|---|
| `tempo/crates/contracts/src/precompiles/mod.rs` | 29-30 | `PATH_USD_ADDRESS` / `DEFAULT_FEE_TOKEN` 常量 |
| `tempo/crates/primitives/src/address.rs` | 11 | `is_tip20_prefix(addr)` 判定函数 |
| `tempo/crates/precompiles/src/lib.rs` | 118 | TIP-20 自动路由 precompile |
| `tempo/crates/precompiles/src/tip_fee_manager/mod.rs` | 201 | Fee manager 处理 TIP-20 fee 的入口 |
| `tempo/crates/precompiles/src/stablecoin_dex/mod.rs` | — | Stablecoin DEX (fee swap 后端) |

# 附录 B：术语表

| 术语 | 含义 |
|---|---|
| **pathUSD** | Tempo 链原生的 USD-pegged TIP-20 stablecoin，地址 `0x20C0000...0000` |
| **TIP-20** | Tempo Improvement Proposal 20，协议级 token 标准（precompile-backed） |
| **TIP-20 prefix** | 12 字节地址前缀 `0x20C0...`，所有 TIP-20 token 地址都带此前缀 |
| **TIP-403** | Tempo 的 transfer policy 标准（黑名单、地区限制等） |
| **StablecoinDEX** | Tempo 的 stablecoin 互换 DEX precompile |
| **Fee Manager** | Tempo gas 处理的核心 precompile（`0xfeec000...`） |
| **Paymaster** | ERC-4337 概念，第三方代支付 gas 的合约 |
| **Approach D** | 移植 Tempo TIP-20 token precompile 到 SCI Chain 的方案 |
| **ISCI20** | SCI Chain 原生归因代币标准（`is IERC20`） |

# 附录 C：相关文档

- `transfer-memo-comparison.docx` —— 四方案对比 + ISCI20 vs MemoLog 深度对比
- `memolog-usdt-security-risks.docx` —— MemoLog 用于外部 ERC-20 的安全风险评估
