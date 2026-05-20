---
title: "SCI Chain MemoLog 方案安全风险评估"
subtitle: "支付 USDT 等外部 ERC-20 的伪造攻击与防御设计"
author: "SCI Chain 工程组"
date: "2026-05-19"
lang: zh-CN
---

# 背景与适用范围

本文聚焦于 SCI Chain **若引入 MemoLog precompile（方案 B）** 用于外部 ERC-20（USDT、USDC 等）支付归因时的**安全风险评估**与**缓解设计**。

## 什么是 MemoLog 方案

MemoLog 是一个 SCI Chain 原生 precompile，提供单一函数 `record(token, from, to, amount, memo)`，仅 emit 事件、不验证任何转账实际发生。设计意图是给**任意 ERC-20**（包括 SCI 自身控制不了的外部代币）提供归因记录能力。

```solidity
interface IMemoLog {
    event TransferWithMemo(
        address indexed token,
        address indexed from,
        address indexed to,
        uint256 amount,
        bytes32 indexed memo
    );
    function record(
        address token, address from, address to,
        uint256 amount, bytes32 memo
    ) external;
}
```

**核心问题**：`record()` 不验证真实转账是否发生，导致**任何人都能伪造任意转账事件**。本文系统化分析由此引发的攻击面，并给出分层防御方案。

## 阅读对象

- SCI Chain 协议工程组
- MPP Gateway 实现/集成方
- 审计 / 财务 / 合规团队
- 接入 SCI Chain 的 dapp / agent 开发者

---

# 1. 风险清单

按攻击者可获利空间从大到小排序，分为三档严重度。

## 1.1 🔴 严重风险（直接经济损失/欺诈）

### R1：凭空伪造转账事件

**攻击场景**

```
Eve（攻击者）调用:
MemoLog.record(USDT_addr, Alice, Bob, 1_000_000_000, "invoice_2026_05_payment")

链上事件: Alice 给 Bob 付了 1000 USDT
真相: USDT 合约里没有任何转账
```

**后果**

- 报销系统按链上事件给 Bob 报销 1000 USDT，实际无转账
- 平台财务对账时多算一笔收入/支出
- Bob 拿假事件去找 Alice 索要"未到账资金"

**攻击门槛**：极低 —— 任何账户花约 \$0.01 gas 即可

**严重度**：⭐⭐⭐⭐⭐

### R2：金额放大攻击

**攻击场景**

```
Alice 真的给 Bob 付了 1 USDT（链上有真 Transfer 事件）
Eve（或 Alice 自己）调用:
MemoLog.record(USDT_addr, Alice, Bob, 100_000_000, "expense_X")

链上看: Alice→Bob 100 USDT (with memo)
真相:   Alice→Bob 1 USDT
```

**后果**

- 简单 cross-check（"是否有同方向 Transfer 事件"）会通过
- 只有精确比对金额才能识破
- 报销系统若按 MemoLog 金额报销 → 直接造假 99 USDT

**严重度**：⭐⭐⭐⭐⭐

### R3：代付身份伪造

**攻击场景**

```
Eve 调用:
MemoLog.record(USDT_addr, Platform_Treasury, Eve, 10_000_000,
               "Q1_2026_salary_payment")

链上看: Platform 给 Eve 发了 10000 USDT 工资
真相:   Platform 不知情
```

**后果**

- Eve 拿这条事件去税务局申报薪资（洗钱合法化）
- Eve 拿这条事件做 KYC（伪造收入证明）
- 平台声誉风险

**严重度**：⭐⭐⭐⭐⭐

## 1.2 🟡 中度风险（误导但难直接获利）

### R4：Token 地址欺骗

**攻击场景**

```
攻击者部署山寨 token 在 0xFAKE
执行真实转账: FAKE.transfer(Bob, 100)
然后调用:    MemoLog.record(REAL_USDT_addr, Eve, Bob, 100, "memo")

事件显示: REAL_USDT 100 token 给 Bob
真相:    FAKE token 100 个给 Bob（这 token 一文不值）
```

**后果**：审计员若仅信 MemoLog 不识别真假 token，会把山寨币当 USDT

**缓解难度**：低（要求审计端验证 token 合约 + 实际 balance 增量）

**严重度**：⭐⭐⭐⭐

### R5：事件重放灌水

**攻击场景**

```
Alice 真的给 Bob 付了 1 USDT
Eve 调用 record(USDT, Alice, Bob, 1, "X") 1000 次（gas 约 5K/次）

链上 1001 条事件，索引器以为有 1001 笔同样转账
```

**后果**：索引器/审计工具如果没 dedup，会把 1 笔算作 1001 笔

**严重度**：⭐⭐⭐

### R6：Memo 撞库 / 抢注

**攻击场景**

```
平台用顺序 jobId 哈希做 memo:
  memo = keccak256("job_" + i)

攻击者预测下一个 jobId,预先注册伪造事件:
  MemoLog.record(USDT, fake_payer, attacker, 1_000_000, keccak256("job_42"))

真实 job_42 完成时,链上已经有一条带正确 memo 的"前置归因"
```

**后果**：归因数据被污染，无法判断真假 record

**缓解**：memo 命名包含随机 nonce 或时间戳

**严重度**：⭐⭐⭐

### R7：Frontrunning 抢归因

**攻击场景**

```
Alice 在 mempool 提交两笔:
  USDT.transfer(Bob, 100)
  MemoLog.record(USDT, Alice, Bob, 100, "X")

Eve 看到, 立即抢先:
  MemoLog.record(USDT, Alice, Eve, 100, "fake")

链上事件顺序:
  1. MemoLog:      Alice→Eve 100 USDT (Eve 抢到)
  2. USDT.Transfer: Alice→Bob 100 USDT
  3. MemoLog:      Alice→Bob 100 USDT (Alice 的)
```

**后果**：审计系统若按 memo 时间序匹配，会把 Alice 的转账归因到 Eve

**缓解**：使用 Alice 自己签的 memo（防 frontrun）

**严重度**：⭐⭐⭐

## 1.3 🟢 低度风险（DoS / 隐私）

### R8：Event Log DoS

- 攻击者批量调 `record()` 灌爆 event log 存储
- 索引器吃不消
- 链节点 sync 变慢
- **缓解**：链层 rate limit 或 minimum gas cost

**严重度**：⭐⭐

### R9：隐私泄露 / 诽谤

- 任何人能在事件里写任意 memo
- 可以恶意把 `"agent_X = real_name_X"` 之类敏感关联写进事件
- 链上不可删除
- **缓解**：教育 + 治理（链上 abuse report 机制）

**严重度**：⭐⭐

### R10：时序伪造

- 攻击者抢在真实转账前 `record()`
- 破坏归因事件的因果链
- 严重度：⭐（多数业务流不依赖严格时序）

---

# 2. 缓解策略

每个策略可单独使用或组合。下面按"实现成本 vs 风险消除"权衡列出。

## 2.1 策略 M1：`msg.sender == from` 强制

最简单的限制：record 只能由 from 本人发起。

```rust
pub fn record(&mut self, calldata: &[u8], msg_sender: Address) -> PrecompileResult {
    let call = IMemoLog::recordCall::abi_decode(calldata)?;
    if msg_sender != call.from {
        return Err(MemoLogError::UnauthorizedSender);
    }
    self.emit_event(...);
    Ok(...)
}
```

| 消除风险 | 引入限制 |
|---|---|
| R1（伪造的 80% 场景） | 失去**代付/平台代记录**能力 |
| R3（代付伪造） | MPP Gateway 无法替 agent 记录归因 |
| R7（frontrun 抢归因） | |

- **代码量**：~5 行
- **遗留风险**：R2、R4
- **适用**：自付场景为主、不需要平台代记录

## 2.2 策略 M2：同 tx Receipt Cross-check（最有效）

precompile 在执行时读取**当前 tx 的 receipt logs**，验证里面有 `(token, from, to, amount)` 匹配的真实 Transfer 事件。

```rust
pub fn record(&mut self, calldata: &[u8], msg_sender: Address) -> PrecompileResult {
    let call = IMemoLog::recordCall::abi_decode(calldata)?;

    // 关键: 从当前 tx receipt 里找匹配的 Transfer 事件
    let receipt_logs = self.storage.current_tx_receipt_logs();
    let transfer_topic = keccak256("Transfer(address,address,uint256)");

    let matched = receipt_logs.iter().any(|log| {
        log.address == call.token
            && log.topics[0] == transfer_topic
            && log.topics[1] == call.from.into_word()
            && log.topics[2] == call.to.into_word()
            && log.data == call.amount.abi_encode()
    });

    if !matched {
        return Err(MemoLogError::NoMatchingTransfer);
    }

    self.emit_event(...);
    Ok(...)
}
```

| 消除风险 | 引入限制 |
|---|---|
| R1, R2, R3, R4, R5, R7 几乎全部严重风险 | 真实 Transfer **必须在 record 之前同 tx 内已发生** |
| | revm 34 / alloy-evm 0.27 不直接暴露 tx receipt 给 precompile —— 需扩展（~300 行） |
| | 跨链桥、proxy contract、internal call 难匹配 |
| | 性能：每次 record 需扫描 receipt（~50μs） |

- **代码量**：~300 行（含 EvmInternals 扩展）
- **效果**：⭐⭐⭐⭐⭐
- **复杂度**：高
- **适用**：高价值/严肃归因场景，可接受技术债

## 2.3 策略 M3：EIP-712 签名授权

调用方提交 `from` 用自己私钥签的 attestation：

```solidity
interface IMemoLog {
    function record(
        address token, address from, address to, uint256 amount,
        bytes32 memo, uint256 nonce,
        bytes calldata signature  // from 用 EIP-712 签的
    ) external;
}
```

```rust
pub fn record(&mut self, calldata: &[u8], _msg_sender: Address) -> PrecompileResult {
    let call = decode(...)?;

    // 1. 验证 EIP-712 签名
    let digest = eip712_typed_data_hash(...);
    let signer = ecrecover(digest, &call.signature)?;
    if signer != call.from {
        return Err(InvalidSignature);
    }

    // 2. 防重放
    if self.used_nonces.read(call.from, call.nonce)? {
        return Err(NonceReused);
    }
    self.used_nonces.write(call.from, call.nonce, true)?;

    self.emit_event(...);
    Ok(...)
}
```

| 消除风险 | 引入限制 |
|---|---|
| R1（伪造 from） | 增加签名/验证 gas（~7K） |
| R3（代付伪造 - from 必须同意） | 需 nonce 持久化（~5K SSTORE） |
| R5（重放） | UX：from 需要离线签名 |
| R7（frontrun） | |

- **代码量**：~200 行
- **遗留风险**：R2、R4
- **适用**：from 是 EOA，业务流允许签名步骤

## 2.4 策略 M4：白名单调用者（中心化信任锚）

```rust
const MPP_GATEWAY: Address = address!("0x...");

pub fn record(&mut self, ..., msg_sender: Address) -> PrecompileResult {
    if msg_sender != MPP_GATEWAY {
        return Err(NotAuthorized);
    }
    // 由 MPP Gateway 做 off-chain 验证
    self.emit_event(...);
    Ok(...)
}
```

| 消除风险 | 引入限制 |
|---|---|
| R1, R3（从可信源 emit） | 信任完全集中在 MPP Gateway |
| | Gateway 被攻破 = 全军覆没 |
| | 中心化（与 SCI Agent-native 哲学略冲突） |

- **代码量**：~10 行
- **适用**：MPP Gateway 是 SCI 协议核心、可信组件
- **可改进**：用 multisig 多签分散信任

## 2.5 策略 M5：经济抑制（rate limit + min gas）

precompile 内置最低 gas 消耗（不论操作多简单），让大规模灌水不经济：

```rust
const MIN_RECORD_GAS: u64 = 50_000;  // 远高于实际成本（防 spam）

pub fn record(...) -> PrecompileResult {
    self.storage.deduct_gas(MIN_RECORD_GAS)?;
    // ... 其余逻辑
}
```

| 消除风险 | 引入限制 |
|---|---|
| R8（DoS） | 正常用户也付更多 gas |

## 2.6 策略 M6：链下 attestation + 链上 evidence 等级标签

事件里增加 `evidenceGrade` 字段，由可信 attester 后置验证：

```solidity
event TransferWithMemo(
    address indexed token, address indexed from, address indexed to,
    uint256 amount, bytes32 indexed memo,
    uint8 evidenceGrade   // 0=unverified, 1=signed, 2=receipt-checked, 3=attested
);
```

- 链下 MPP Gateway / 审计服务监听事件，做 cross-check 后用合约 attest
- UI / 审计工具按等级展示：✅ verified / ⚠️ unverified

| 消除风险 | 引入限制 |
|---|---|
| 不消除风险本身，但**让风险可见可识别** | 需要链下基础设施配合 |

---

# 3. 推荐的加固方案组合

针对 SCI Chain 支付 USDT 的场景，分四层防御：

## Layer 1（链层强约束，必做）

**M1（`msg.sender == from`） + M5（rate limit）**

```rust
pub fn record(&mut self, calldata: &[u8], msg_sender: Address) -> PrecompileResult {
    self.storage.deduct_gas(MIN_RECORD_GAS)?;   // M5

    let call = IMemoLog::recordCall::abi_decode(calldata)?;
    if msg_sender != call.from {                 // M1
        return Err(MemoLogError::UnauthorizedSender);
    }

    self.emit_event(...);
    Ok(...)
}
```

- **消除**：R1、R3、R7、R8
- **保留**：R2、R4、R5（重放问题需配合 nonce）
- **代码量**：~15 行

## Layer 2（业务层强约束，应做）

**M3（EIP-712 + nonce）** —— 允许代付/平台代记录场景

```solidity
function record(
    address token, address from, address to, uint256 amount,
    bytes32 memo, uint256 nonce, bytes calldata fromSig
) external;
```

- **消除**：R1（伪造 from）、R3（代付伪造）、R5（重放）
- **增加 ~7K gas + ~5K SSTORE**（nonce 持久化）
- **代码量**：~200 行

## Layer 3（链下审计，必做）

**M2（off-chain 版） + M6（evidence grade）**

```typescript
// MPP Gateway 监听事件
on TransferWithMemo(token, from, to, amount, memo):
    txLogs = eth_getTransactionReceipt(eventTxHash).logs
    realTransfer = txLogs.find(log =>
        log.address == token
        && log.topics[0] == TRANSFER_SIG
        && decodeTransfer(log) == {from, to, amount})

    if realTransfer:
        evidenceGrade = "atomic"          // 同 tx 内匹配,可信度高
    else if existsTransferInRecentBlocks(token, from, to, amount):
        evidenceGrade = "fuzzy_matched"   // 邻近块内有匹配,中等可信
    else:
        evidenceGrade = "claim_only"      // 仅 memo 无匹配转账,危险
        ALERT(memo, from, to)             // 报警

    storeAttribution(memo, evidenceGrade, ...)
```

- **作用**：让 R2、R4 在审计层暴露出来
- **代码量**：~500 行（off-chain，TS/Python）

## Layer 4（终极防御，可选）

**M2（链上版） —— 同 tx receipt cross-check**

- 工作量大但效果最好
- 适合 SCI 想做"协议级 evidence-grade"承诺的场景

---

# 4. 加固版 MemoLog 完整实现样例

## 4.1 ABI

```rust
// sci/crates/precompile-abi/src/precompiles/memo_log.rs
crate::sol! {
    interface IMemoLog {
        event TransferWithMemo(
            address indexed token, address indexed from, address indexed to,
            uint256 amount, bytes32 indexed memo,
            uint8 evidenceGrade   // 0=signed, 1=receipt-checked
        );

        // 自付模式: msg.sender == from, 无需签名
        function recordSelf(
            address token, address to, uint256 amount, bytes32 memo
        ) external;

        // 代付模式: 任何人可提交, 但 from 必须签
        function recordSigned(
            address token, address from, address to, uint256 amount,
            bytes32 memo, uint256 nonce, bytes calldata signature
        ) external;

        // 严格模式: 要求同 tx 内有匹配 Transfer 事件
        function recordVerified(
            address token, address to, uint256 amount, bytes32 memo
        ) external;

        error UnauthorizedSender();
        error InvalidSignature();
        error NonceReused(uint256 nonce);
        error NoMatchingTransfer();
    }
}
```

## 4.2 Precompile 实现

```rust
// sci/crates/precompiles/src/memo_log/mod.rs
const MIN_RECORD_GAS: u64 = 30_000;

#[contract(addr = MEMO_LOG_ADDRESS)]
pub struct MemoLog {
    // used_nonces[from][nonce] -> bool（防重放）
    used_nonces: Mapping<Address, Mapping<U256, bool>>,
}

impl MemoLog {
    pub fn record_self(&mut self, msg_sender: Address, call: recordSelfCall) -> Result<()> {
        self.storage.deduct_gas(MIN_RECORD_GAS)?;
        // M1: msg.sender 即为 from
        self.emit_transfer_with_memo(
            call.token, msg_sender, call.to, call.amount, call.memo,
            0  // evidenceGrade = signed (隐式 by msg.sender)
        )?;
        Ok(())
    }

    pub fn record_signed(&mut self, _msg_sender: Address, call: recordSignedCall) -> Result<()> {
        self.storage.deduct_gas(MIN_RECORD_GAS)?;

        // 防重放
        if self.used_nonces[call.from][call.nonce].read()? {
            return Err(MemoLogError::NonceReused(call.nonce).into());
        }
        self.used_nonces[call.from][call.nonce].write(true)?;

        // M3: EIP-712 签名验证
        let digest = self.eip712_digest(
            call.token, call.from, call.to, call.amount, call.memo, call.nonce
        );
        let recovered = self.storage
            .recover_signer_eip712(digest, &call.signature)?
            .ok_or(MemoLogError::InvalidSignature)?;
        if recovered != call.from {
            return Err(MemoLogError::InvalidSignature.into());
        }

        self.emit_transfer_with_memo(
            call.token, call.from, call.to, call.amount, call.memo,
            0  // evidenceGrade = signed
        )?;
        Ok(())
    }

    pub fn record_verified(&mut self, msg_sender: Address, call: recordVerifiedCall) -> Result<()> {
        self.storage.deduct_gas(MIN_RECORD_GAS)?;

        // M2: 同 tx receipt cross-check（需协议支持）
        let receipt_logs = self.storage.current_tx_receipt_logs()?;
        let transfer_sig = keccak256("Transfer(address,address,uint256)");
        let matched = receipt_logs.iter().any(|log| {
            log.address == call.token
                && log.topics[0] == transfer_sig
                && log.topics[1] == msg_sender.into_word()
                && log.topics[2] == call.to.into_word()
                && decode_u256(&log.data) == call.amount
        });

        if !matched {
            return Err(MemoLogError::NoMatchingTransfer.into());
        }

        self.emit_transfer_with_memo(
            call.token, msg_sender, call.to, call.amount, call.memo,
            1  // evidenceGrade = receipt-checked
        )?;
        Ok(())
    }
}
```

## 4.3 三种模式的语义对比

| 模式 | 调用者 | from 来源 | 验证机制 | evidenceGrade | 适用场景 |
|---|---|---|---|---|---|
| `recordSelf` | from 本人 | `msg.sender` | 无（信任 msg.sender） | 0 (signed) | Agent 自付场景 |
| `recordSigned` | 任意调用方 | calldata | EIP-712 签名 + nonce | 0 (signed) | MPP Gateway 代发 |
| `recordVerified` | from 本人 | `msg.sender` | 同 tx receipt cross-check | 1 (receipt-checked) | 严格审计场景 |

---

# 5. 风险/缓解矩阵

| 风险 | 严重度 | M1 self | M3 signed | M2 verified | M4 whitelist | 链下 cross-check |
|---|---|---|---|---|---|---|
| R1 凭空伪造 | 🔴 | ✅ | ✅ | ✅ | ✅ | ✅ |
| R2 金额放大 | 🔴 | ❌ | ❌ | ✅ | ⚠️ | ✅ |
| R3 代付伪造 | 🔴 | ✅ | ✅ | ✅ | ⚠️ | ✅ |
| R4 Token 欺骗 | 🟡 | ❌ | ❌ | ✅ | ⚠️ | ✅ |
| R5 重放 | 🟡 | ❌ | ✅ | ✅ | ⚠️ | ✅ |
| R6 撞库 | 🟡 | ⚠️* | ⚠️* | ⚠️* | ⚠️* | ⚠️* |
| R7 Frontrun | 🟡 | ✅ | ✅ | ✅ | ✅ | ⚠️ |
| R8 DoS | 🟢 | M5 必配套 | M5 必配套 | 自动防 | 自动防 | — |

\* R6（撞库）必须靠 memo 命名规范防（带随机 nonce/时间戳）

图例：
- ✅ 完全消除
- ⚠️ 部分缓解或依赖配合
- ❌ 未消除

---

# 6. 实操建议

## 6.1 给 SCI Chain 引入 MemoLog 支付 USDT 的最小可行配置

### 链层 precompile 实现三种模式

- `recordSelf` —— 自付（M1，最常用）
- `recordSigned` —— 代付带签名（M3，平台代记录）
- `recordVerified` —— 严格模式（M2，留作未来扩展，先 stub）

### MPP Gateway 配套

- 监听 MemoLog 事件
- 链下做 receipt cross-check
- 把 `evidenceGrade` 增量计算后存到归因数据库
- 报警/拒绝异常 record（金额异常、from 在黑名单等）

### 审计 / UI 工具

- 强制按 `evidenceGrade` 分级展示
- **永远不**单凭 `recordSelf` / `recordSigned` 的事件给用户报销
- 报销/财务流程必须等 MPP Gateway 完成 cross-check 标记为 `verified` 才放行

### 关键工程约束

- **MemoLog 事件 ≠ 转账证据**（必须配合 Transfer 事件）
- 文档明确写出这一点，让所有集成方知道
- 错误传达成本极高（误信会有重大损失）

## 6.2 部署 / 上线 Checklist

- [ ] precompile 实现 + 单元测试通过（覆盖 `recordSelf` / `recordSigned` 至少 90% 路径）
- [ ] EIP-712 domain separator 固定，记录在 CLAUDE.md 与 SDK 文档
- [ ] `MIN_RECORD_GAS` 数值确定（建议 30K-50K，按 SCI 链 gas 经济决定）
- [ ] MPP Gateway 实现 receipt cross-check，attestation 入库
- [ ] 审计/UI 工具实现 evidenceGrade 分级展示
- [ ] Agent SDK 集成（封装 `recordSelf` / `recordSigned` 两种调用）
- [ ] 文档明确警告 "MemoLog 事件 ≠ 转账证据"
- [ ] 渗透测试 / red team 演练：尝试上述 R1-R10 攻击
- [ ] 监控大盘：record 频率、金额分布、from 黑名单命中、evidenceGrade 分布

## 6.3 长期演进路线

| 阶段 | 改进 | 时间窗 |
|---|---|---|
| Phase 1 (M0) | 加固版 MemoLog 上线（M1 + M3 + M5） | 上线时 |
| Phase 2 (M3) | MPP Gateway off-chain cross-check 与 evidenceGrade 全链路接入 | 上线 3 个月内 |
| Phase 3 (M6) | 评估同 tx receipt cross-check 的实施成本（M2 链上版） | 上线 6 个月内 |
| Phase 4 | 推动 USDT/USDC 等头部 stablecoin 在 SCI Chain 部署 SCI20-aware wrapper（如 `sUSDT`），重要支付走方案 A 路径 | 长期 |

---

# 7. 结论

**裸用 MemoLog 不可接受** —— 任何人都能伪造任何转账事件，对 USDT 等外部代币会带来真实经济损失。

**必须叠加 M1 + M3 + 链下 cross-check 三层防御**才能用。即便如此，外部 ERC-20 的归因强度始终低于 SCI20 原生的 `transferWithMeta` —— 这是结构性 trade-off。

针对 SCI Chain 上 USDT 等外部代币支付的具体建议：

- **短期**：用加固版 MemoLog（M1 + M3）+ MPP Gateway off-chain cross-check（M2 链下版）
- **中期**：评估 M2 同 tx receipt cross-check 的实施成本，做 protocol-level 增强
- **长期**：推动 USDT 等头部 stablecoin 在 SCI Chain 上部署 SCI20-aware wrapper 合约（如 `sUSDT`），让重要支付走 ISCI20 路径，享受 atomic evidence

---

# 附录 A：EIP-712 Domain Separator 设计

```solidity
EIP712Domain {
    name:    "SCI Chain MemoLog",
    version: "1",
    chainId: 42001,
    verifyingContract: 0x4D454D4F00000000000000000000000000000000   // MEMO_LOG_ADDRESS
}

TypeHash:
  keccak256("Record(address token,address from,address to,uint256 amount,bytes32 memo,uint256 nonce)")
```

# 附录 B：术语表

| 术语 | 含义 |
|---|---|
| **MemoLog** | SCI Chain 独立 precompile，emit 转账归因事件 |
| **MPP** | Machine Payments Protocol，Agent 间使用的支付协议（HTTP 402 扩展） |
| **MPP Gateway** | MPP 的中心化网关，负责 off-chain attestation |
| **ISCI20** | SCI Chain 原生归因代币标准（is IERC20） |
| **evidenceGrade** | 归因数据可信度等级：0/1/2/3 |
| **atomic evidence** | 同笔交易内原子完成的归因（强证据） |
| **claim evidence** | 仅链上声明的归因（需 cross-check） |
| **session key / access key** | 受限的次级签名密钥，受 AccountKeychain 管理 |
| **EIP-712** | 以太坊结构化数据签名标准 |

# 附录 C：相关文档

- [SCI Chain 转账附加信息方案对比分析](./transfer-memo-comparison.docx) —— 四方案对比 + ISCI20 vs MemoLog 深度对比
- CLAUDE.md —— SCI Chain 开发指南，含 AccountKeychain 与 keychain 集成
