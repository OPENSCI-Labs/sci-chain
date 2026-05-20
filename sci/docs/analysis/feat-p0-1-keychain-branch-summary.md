---
title: "feat/p0-1-keychain 分支改动总览与修改原因"
date: "2026-05-21"
branch_base: "3049ce2e3 fix: error severity for invalid FC state -> reset (#2551)"
branch_head: "7ac6fa65e sci: add devnet genesis patches + compose override for keychain"
commits: 4
files_changed: 70
lines_added: 21840
lines_removed: 26
---

# feat/p0-1-keychain 分支改动总览

本文档把 `feat/p0-1-keychain` 分支从 base v0.8.0 主线分叉以来的所有改动，按
"为什么需要"的视角分类列出。它的目的是让审稿人 / 后续开发者快速回答：

> "SCI fork 改了 Base 的哪些地方？为什么必须改？哪些是新加的、不影响 Base？"

阅读它之后应该清楚：**SCI 对 Base 的修改面是极小且明确受控的**——只动了 EVM 工厂
+ 处理器装配这一条路径上的 6 个 Base 文件（修改 + 1 个新增），其余全部 21000+ 行
都在 `sci/` 目录下，与 Base 主线零冲突。

## 0. 提交清单

| Commit | 标题 | 主要目的 |
|---|---|---|
| `98df2b6ec` | sci: scaffold sci/ workspace directory structure | 占位 sci/ 目录树（`.gitkeep` + 顶级 README），让后续 SCI 工作有结构归属 |
| `f08aaa3a5` | sci: replace CLAUDE.md with SCI Chain development guide | 把 base 原版 CLAUDE.md 替换为 SCI 开发指南，写清架构、关键规则、构建命令 |
| `ef4914ea8` | sci: port keychain precompile and wire pre-execution hook | 从 Tempo v1.6.0 移植 keychain 精灵到 SCI，并在 Base EVM factory 接入 pre-execution hook |
| `7ac6fa65e` | sci: add devnet genesis patches + compose override for keychain | devnet 集成测试发现的 genesis alloc gap 修复 + image tag 隔离方案 |

## 1. 改动总规模

```
70 files changed, 21840 insertions(+), 26 deletions(-)
```

按文件类别拆解：

| 类别 | 文件数 | 行数（新增） |
|---|---|---|
| Base 文件修改（保留原结构，最小化改动） | 6 | ~50 |
| Base 文件新增（仅 1 个，归在 base-common-evm crate） | 1 | 173 |
| CLAUDE.md（重写） | 1 | +536 / -353（净 +183） |
| Cargo.toml / Cargo.lock（工作区注册 sci-* crates） | 2 | ~454 |
| `sci/crates/precompiles/`（keychain 业务代码 + 存储抽象 + 测试） | 24 | ~16000 |
| `sci/crates/precompiles-macros/`（Storable / contract proc 宏） | 7 | ~3300 |
| `sci/crates/precompile-abi/`（ABI 接口 sol! 绑定） | 10 | ~500 |
| `sci/crates/tempo-chainspec-shim/`（兼容垫片） | 2 | ~98 |
| `sci/devnet/`（devnet 测试配置） | 4 | ~108 |
| `sci/docs/`（开发文档） | 1 | 589 |
| `sci/contracts/` / `sci/gateway/`（占位目录） | 9 | 0（仅 .gitkeep） |

---

## 2. Base 文件改动（**仅 7 个，CLAUDE.md Critical Rule #2 红线**）

CLAUDE.md 在 `sci:` namespace convention 一节明确约定 "ONLY 7 Base files are touched"。
这 7 个文件全部围绕**同一个工程目标**：在 Base 的 EVM 装配路径上，**在不替换、不
fork 任何上游模块的前提下**，把 SCI 精灵和 pre-execution hook 插进去。

### 2.1 `Cargo.toml`（工作区根）

**改动**：
- `workspace.members` 加入 `sci/crates/precompiles`、`sci/crates/precompiles-macros`、
  `sci/crates/precompile-abi`、`sci/crates/tempo-chainspec-shim`
- `workspace.dependencies` 加入对应四个 crate（`sci-precompiles`、`sci-precompiles-macros`、
  `sci-precompile-abi`、`tempo-chainspec-shim`），并配置 `package = "..."` 重命名
  让 ported Tempo 业务源码可以原样用 `tempo_*` 路径引用

**为什么必须改**：
- Cargo workspace 是 Rust 工程的硬约束——SCI crate 必须列在 root members 才能参与
  workspace 锁版本管理 / 共享 target/
- `package = "..."` 重命名是**实现"Tempo 源码 verbatim 移植"的核心机制**：上游
  Tempo 用 `tempo_chainspec::hardfork::TempoHardfork` 等路径，SCI 实际的 crate 名是
  `sci-precompiles` / `tempo-chainspec-shim` 等，重命名让两边统一无需改源码

### 2.2 `Cargo.lock`

**改动**：294 行（新依赖解析）

**为什么必须改**：lock 文件随 Cargo.toml 变化必然更新，确定性构建必须 commit

### 2.3 `crates/common/evm/Cargo.toml`

**改动**：加 `sci-precompiles.workspace = true` 一行

**为什么必须改**：
- Base 的 EVM 工厂 (`base-common-evm` crate) 需要调用 `sci_precompiles::install(...)`
  来把 SCI 精灵注册进 PrecompilesMap
- 这是 Base 跟 SCI 唯一的工程依赖边界

### 2.4 `crates/common/evm/src/factory.rs`

**改动**：14 行净增。在 `BaseEvmFactory::create_evm` 和 `create_evm_with_inspector`
两个构造点，于 `PrecompilesMap::from_static(...)` 之后立刻调用
`sci_precompiles::install(&mut precompiles, &input.cfg_env)`

**为什么必须改**：
- Base v0.8 的 EVM 装配把 Ethereum 标准 precompile 写死在 `BasePrecompiles::new_with_spec`
  里。SCI 精灵地址（0xAAAA...0000 / 0xAAAA...0001）不在那个集合内，必须在装配阶段
  加进去
- 选择"装配后 install"而非"fork 一个 SciPrecompiles 集合"是因为：① 改动面更小，
  ② 上游 BasePrecompiles 变化时不会有冲突，③ install 只是注册查找回调，未来加新
  SCI 精灵不需要再改 Base

### 2.5 `crates/common/evm/src/lib.rs`

**改动**：3 行净增——`mod sci_handler; pub use ...::SciHandler;`

**为什么必须改**：
- 新加的 `sci_handler.rs`（见 §2.6）需要被 lib.rs 公开，`exec.rs` 才能 import `SciHandler`
- 之所以让 `sci_handler.rs` 在 base-common-evm 而不是 sci-precompiles：**避免 cycle 依赖**。
  sci-precompiles 已经依赖 base-common-evm（用其类型），反过来不可能。详细论证在
  `sci/crates/precompiles/src/handler/mod.rs` 文件头注释

### 2.6 `crates/common/evm/src/sci_handler.rs`（**新增的唯一 Base 文件**）

**173 行新代码**。`SciHandler<EVM, ERROR, FRAME>` 包装 `OpHandler`，所有 Handler trait
方法逐字 delegate 给内部的 `OpHandler`，**除了** `validate_against_state_and_deduct_caller`：
后者先检查 `tx_type == DEPOSIT_TRANSACTION_TYPE`（OP-Stack 预编译 tick 路径），
不是 deposit 才调 `sci_precompiles::run_pre_execution_hook` 应用 keychain 检查。

**为什么必须新增**：
- pre-execution hook（CircuitBreaker → Scope → SpendingLimit）必须在 EVM 主体执行
  **之前**触发：要在 gas 预扣、nonce 增加之后，但在每个 call 真正执行之前，才能
  fail-fast 不浪费 gas
- revm 提供 Handler trait 作为这种 hook 的唯一干净切入点
- 用 wrapper 而非直接 fork OpHandler：保持上游 OpHandler 改动可见、可 review，
  我们只动 1 个方法
- system-call 路径（包括 deposit tx 和 OP-Stack 系统调用）通过 `tx_type` 早退，
  保证 OP-Stack 预编译 tick 不受 SCI hook 干扰

### 2.7 `crates/common/evm/src/api/exec.rs`

**改动**：24 行净改。5 处 `OpHandler::<_, _, EthFrame<EthInterpreter>>::new()` 构造
点全部替换为 `SciHandler::<_, _, EthFrame<EthInterpreter>>::new()`：
- `transact_one`
- `replay`
- `inspect_one_tx`
- `system_call_one_with_caller`
- `inspect_one_system_call_with_caller`

同时给 `OpContextTr` 定义加两个 trait bound：`Db: alloy_evm::Database`、`Journal: Debug`。

**为什么必须改**：
- exec.rs 是 Base 真正实例化 EVM 处理器的位置。SCI hook 要起作用就必须在这里
  用 `SciHandler` 包装
- 5 处都换是为了一致性：任何执行路径都走 SCI handler。其中 system-call 路径靠
  `sci_handler.rs` 内部对 `tx_type` 的判断早退，无需在 exec.rs 区分
- 两个 trait bound 是 `SciHandler` 内部构造 `EvmInternals` 时的硬要求，
  所有 Base 具体 context 类型都已满足，是无害扩展

### 2.8 `CLAUDE.md`（重写）

**改动**：+536 / -353 行。把 base 原版 CLAUDE.md 替换为 SCI 开发指南。

**为什么必须改**：
- SCI fork 跟 base 主线开发流程不同——chain ID 42001、不允许 fmt Base 文件、SCI 命名
  规范、Tempo 同步流程、devnet 红线等等
- 把这些规则集中写在 CLAUDE.md，作者无需逐条解释，AI 协作者也能直接遵循
- 保留了 Base 原版的核心代码风格规则（"Base Upstream Style Rules"一整节）作为继承

### 2.9 `etc/docker/devnet-env`（**未在本次 diff 列表**，已在 base scaffold 阶段 commit）

按 CLAUDE.md "Critical Rules #2" 应该列在 7 个之内。它修改 Chain ID 为 42001。
**本分支未直接改动它**——它在更早的 base scaffold 阶段就改过，merge-base 之前。本分支
里没出现在 diff 中。

---

## 3. SCI Rust crates（全部在 `sci/crates/` 下，与 Base 零交叉）

四个独立 crate，组成 keychain 精灵的完整实现。

### 3.1 `sci/crates/precompiles/`（~16,000 行）

**核心 crate，所有精灵业务逻辑 + EVM 后端存储抽象 + 集成测试**。

| 模块 | 行数 | 用途 |
|---|---|---|
| `account_keychain/mod.rs` | 4328 | Keychain 业务核心：authorize / revoke / spending limits / call scopes |
| `account_keychain/dispatch.rs` | 365 | ABI selector 路由（包含 T3 hardfork 调度） |
| `account_keychain/sci_ext.rs` | 38 | **SCI-only 扩展**：公开 `key_is_active` 给 hook 用（Tempo upstream 是 crate-private） |
| `sci_agent_state/mod.rs` | 227 | **SCI-only 第二个精灵**：CircuitBreaker trip 状态 |
| `sci_agent_state/dispatch.rs` | 63 | 同上的 ABI 路由 |
| `storage/evm.rs` | 699 | EVM 后端存储 provider（生产路径） |
| `storage/hashmap.rs` | 284 | InMemory 后端（单元测试用） |
| `storage/packing.rs` | 1180 | 字段打包/解包逻辑（让 sigType+expiry+enforce+revoked 共享一个 slot） |
| `storage/thread_local.rs` | 587 | StorageCtx 线程局部 + 各种 enter\_\* 入口 |
| `storage/types/{mapping,vec,set,array,slot,bytes_like,primitives,mod}.rs` | ~6300 | Storable 类型系统（Mapping<K,V>、Vec<T>、Set、定长 Array、字节串、原子类型） |
| `handler/hook.rs` | 294 | **pre-execution hook 主逻辑**（包括 7702 delegation 检测、batch decode、CB 检查、scope + spending 检查、checkpoint 回滚） |
| `handler/decode.rs` | 204 | 解码 `SCIAgentDelegator::execute(Call[])` 的 calldata |
| `handler/mod.rs` | 25 | Public API: `run_pre_execution_hook` / `apply_post_execution_deductions` |
| `error.rs` | 283 | `SciPrecompileError` 错误枚举 + `IntoPrecompileResult` |
| `lib.rs` | 304 | `Precompile` trait、`install(...)`、`sci_precompile!` 宏、`SelectorSchedule`、`dispatch_call` |
| `test_util.rs` | 130 | TIP20Setup stub（Tempo upstream 有真实 TIP-20 工厂，SCI 用 ERC-20，因此 stub） |
| `tests/hook_e2e.rs` | 679 | **14 个 hook 端到端集成测试**（强 R1、batch 部分失败、CB 等） |

**为什么必须存在**：
- 这是 SCI 的核心增值——AI 代理 keychain。没有这些代码，SCI 就跟 base v0.8 没区别
- 80% 是从 Tempo v1.6.0 verbatim 移植过来（业务源码 + macros + storage 抽象），
  剩下 20% 是 SCI 特有：
  - `account_keychain/sci_ext.rs`：SCI 公开了 `key_is_active`（Tempo 内部用）给 hook
  - `sci_agent_state/`：Tempo 没有的、SCI 独有的 CircuitBreaker 状态精灵
  - `handler/`：Tempo 在 `revm/src/handler.rs` 写 hook，SCI 因为 reverse-import 不可（避免
    cycle）放在这里
  - alloy 路径调整：Tempo 用 `alloy` 伞包，base 用单 crate（alloy_primitives 等），所有
    ported 文件做了 path 调整

### 3.2 `sci/crates/precompiles-macros/`（~3,300 行）

`#[contract]` 和 `#[derive(Storable)]` proc-macro 的实现，**verbatim 从 Tempo 移植**。

| 文件 | 用途 |
|---|---|
| `lib.rs` | 宏入口 |
| `storable.rs` | `#[derive(Storable)]` 的核心实现 |
| `storable_primitives.rs` | 各种原始类型的 Storable impl |
| `storable_tests.rs` | 宏内部测试 |
| `layout.rs` | 存储布局分析 |
| `packing.rs` | 字段打包计算 |
| `utils.rs` | proc-macro 通用辅助 |

**为什么必须存在**：
- `#[contract(addr = ACCOUNT_KEYCHAIN_ADDRESS)]` 把一个 Rust struct 转化成 EVM 存储槽
  布局，类似 Solidity 的 storage layout
- `#[derive(Storable)]` 自动生成 read/write/checkpoint 等 trait 实现
- 没有这两个宏，keychain 业务代码就必须手写所有存储槽访问，~10x 代码量且易错
- 这两个宏的逻辑很多（packing / layout 算法），所以 3300 行不算多

### 3.3 `sci/crates/precompile-abi/`（~500 行）

ABI 接口定义，用 alloy `sol!` 宏声明 Solidity interface，让 Rust / TypeScript 共享同一份
ABI 定义。

| 文件 | 用途 |
|---|---|
| `precompiles/account_keychain.rs` | `IAccountKeychain` interface（authorizeKey / revokeKey / getKey 等） |
| `precompiles/sci_agent_state.rs` | `ISciAgentState` interface（tripKey / isTripped 等） |
| `precompiles/common_errors.rs` | 公共 error 类型 |
| `precompiles/tip20.rs` | TIP-20 接口（SCI 用作 selector 标识，不实现） |
| `predeploys/erc20.rs` | ERC-20 接口（spending limit 内部用） |
| `predeploys/sci_agent_delegator.rs` | `SCIAgentDelegator::execute(Call[])` 接口（hook 用来解 calldata） |

**为什么必须存在**：
- 把 ABI 定义集中在一个 crate，dispatch.rs / hook decode / 外部消费者都从这里 import
- alloy `sol!` 编译期生成类型安全的 encode/decode 代码，比手写 ABI 解析更可靠

### 3.4 `sci/crates/tempo-chainspec-shim/`（~98 行）

30 行 Cargo.toml + 84 行 lib.rs 的最小垫片 crate，对外暴露 `tempo_chainspec::hardfork::TempoHardfork`
以及 SCI-facing alias `SciHardfork`。

**为什么必须存在**：
- Tempo 上游有完整的 `tempo_chainspec` crate（chainspec + hardfork + genesis 全套）
- SCI 不需要这些——SCI 用 base 的 chainspec
- 但 ported Tempo 业务源码会 `use tempo_chainspec::hardfork::TempoHardfork`
- shim 让这些 use 语句继续编译通过，**让 Tempo 源码可以 verbatim 移植**

是"verbatim 移植"策略的关键工程件之一。

---

## 4. SCI Devnet 测试配置（`sci/devnet/`，本次新增的 4 个文件）

| 文件 | 行数 | 用途 |
|---|---|---|
| `docker-compose.sci.yml` | 29 | Compose override，把 `base-client.image` / `base-builder.image` 指向 `:sci` |
| `sci-allocs.json` | 12 | SCI 精灵地址的 genesis alloc（`{nonce:0, balance:0, code:"0xef"}`） |
| `apply-sci-allocs.sh` | 67 | jq 合并脚本：把 sci-allocs.json 合并到 op-deployer 生成的 genesis.json |
| `.gitkeep` | 0 | 占位 |

**为什么必须新增**（这次 session 才发现的）：
- SCI 精灵地址（0xAAAA...0000 / 0xAAAA...0001）不在 op-deployer 默认 genesis 里
- revm 把没有 alloc 的地址视为 empty account，EIP-161 会在 tx 结束时 GC 整个账户
- 没有 alloc → keychain sstore 看似成功（event 出来）但 storage 被丢弃 → 任何 stateful
  测试（T4 authorizeKey 等）失败
- Tempo 上游 dev.json 用 `code: "0xef"` 占位解决，SCI 沿用这个模式
- compose override 是 image tag 隔离方案的一部分（`:sci` 不污染 `:local`），让 SCI 测试和
  base 测试可以共存于同一 devnet 主机

详细的根因 + 调试过程 + 完整环境改动清单见 [`sci/docs/analysis/devnet-p0-1-test-report-2026-05-21.md`](analysis/devnet-p0-1-test-report-2026-05-21.md)。

---

## 5. SCI 文档（`sci/docs/`）

| 文件 | 行数 | 用途 |
|---|---|---|
| `analysis/devnet-p0-1-test-report-2026-05-21.md` | 589 | 本次 devnet 集成测试报告：完整过程、3 个 blocker、修复方案、可复现工作流 |

**为什么必须新增**：
- 测试踩的 3 个坑（dev profile reth panic、EIP-161 alloc gap、rollup.json hash drift）
  不是显而易见的，每一个都阻塞过功能验证
- 没有报告，下次启动 devnet 还会再踩一遍
- 报告 §6 提供"从零起 devnet 的完整工作流"，是新人 / 新机器的 onboarding 资料

---

## 6. 占位目录（`.gitkeep` × 9）

`sci/contracts/{script,src/agent,src/integration,src/interfaces,test}`、
`sci/crates/contracts/abi`、`sci/docs/api`、`sci/gateway/src/{core,mpp,rest}`。

**为什么必须存在**：
- CLAUDE.md 的 "Repository Structure" 一节定义了 sci/ 完整目录树
- 这些目录预留给后续工作：Solidity 合约（Heath 的范围）、Gateway（TS）、API 文档等
- `.gitkeep` 让空目录被 git 追踪，结构稳定

---

## 7. 改动原则归纳

把"为什么必须改"再抽象一层，本分支遵守的 4 个工程原则：

### 原则 1：Base 文件改动最小化（hard-capped 7 个）

**为什么**：
- 让上游 Base merge 永远只有一种冲突可能：那 7 个文件
- AI 协作者 / 新人不会"顺手"改 Base 文件（CLAUDE.md 明文禁止）
- 任何新的 Base 改动都需要更新 CLAUDE.md 的"7 files"清单并 justify

### 原则 2：用 Cargo `package = ...` 重命名实现 Tempo verbatim 移植

**为什么**：
- 上游 Tempo 是 SCI 重要的 keychain 来源，会持续演进
- 如果对源码做 identifier 重命名（`tempo_*` → `sci_*`），每次 Tempo 升级都是大量
  merge conflict
- Cargo 重命名让源码原文不动，只在 workspace Cargo.toml 一个地方做映射
- 唯一不能 verbatim 的是 alloy 路径差异（Tempo 用伞包，base 用单 crate），那是 path
  替换、可一次性 sed 修复

### 原则 3：SCI 特有的偏离都明确写在 CLAUDE.md "Critical Rules"

**为什么**：
- 比如 `is_tip20()` stub 返回 true、`test_util::TIP20Setup` no-op、ignored test 列表
- 这些偏离是 SCI 设计决策（不是 bug），写下来让以后 merge upstream 时知道哪些不能跟

### 原则 4：所有 SCI 新增放 `sci/` 目录，零 Base 目录污染

**为什么**：
- 跟原则 1 一致，给 reviewer 一个清晰边界
- Base reviewer 看 diff 时只看 7 个 Base 文件 + sci/ 目录新增，不会被海量代码淹没
- 对 base merge：只要 7 个文件没 conflict，整个 PR 就 merge 干净

---

## 8. 验证

整个分支当前的状态（截至 commit `7ac6fa65e`）：

- **本地单元测试**：307 lib + 14 hook_e2e + 74 macro 测试全过
- **远端 cargo check / test**：同上，通过（见报告 Phase A）
- **devnet hot-swap**：base-client + base-builder 跑 `:sci` image，0 panic
- **devnet 功能 T1-T6**：全部 PASS（详见报告 §3）
- **CI / lint**：未跑（CI 配置不属于本次范围）

---

## 9. 仍 blocked / 范围外

- **T7 长跑稳定性**：可选，未跑
- **T8 完整 agent-tx 闭环**：等 Heath 落地 `SCIAgentDelegator.sol` 部署到
  `0xCCCC...01`，并给该地址加 genesis alloc。已经在内存测试（14 个 `hook_e2e`）里
  完整验证过路径
- **MPP Gateway**：`sci/gateway/` 当前是占位，不在 P0-1 范围
- **SCI mainnet chainspec**：仅 devnet 模板存在，主网 chainspec 是 P1 任务

---

## 10. 一句话总结

`feat/p0-1-keychain` 用 **7 个 Base 文件改动（其中 1 个新增）** + **~22000 行 sci/ 新增**
完成了 P0-1 Keychain 精灵的端到端落地。Base 改动局限在 EVM 工厂 / handler 装配这一条路径，
上游 merge 友好；SCI 业务代码 80% 是 Tempo verbatim 移植，靠 Cargo `package` 重命名机制
保持可追踪；devnet 测试发现并修复了 EIP-161 alloc gap 等 3 个非平凡 blocker，
全部沉淀到 `sci/devnet/` 和 `sci/docs/`。
