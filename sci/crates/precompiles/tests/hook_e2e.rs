//! Integration tests for the SCI Chain pre-execution hook.
//!
//! These drive [`SciHandler`] through a real [`BaseEvm`] backed by [`InMemoryDB`] to
//! verify behaviors that unit tests against `HashMapStorageProvider` can't reach:
//!
//! - 7702 delegation parsing on real `Bytecode::Eip7702` values
//! - `enter_keychain_storage` under revm's borrow model
//! - journal auto-rollback on tx revert (Q4 R1)
//! - `BaseHandler` ↔ `SciHandler` wrapper interplay
//! - deposit / `TxKind::Create` short-circuit paths

use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address};
use alloy_sol_types::SolCall;

use base_common_chains::BaseUpgrade;
use base_common_evm::{BaseEvm, BaseEvmFactory, BaseHaltReason, BaseSpecId, BaseTransaction};
use revm::{
    DatabaseCommit,
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv, TxEnv},
    context_interface::result::{ExecutionResult, Output},
    database::InMemoryDB,
    database_interface::Database,
    inspector::NoOpInspector,
    primitives::KECCAK_EMPTY,
    state::AccountInfo,
};

use sci_precompiles::{ACCOUNT_KEYCHAIN_ADDRESS, SCI_AGENT_STATE_ADDRESS};
use tempo_contracts::{
    precompiles::{
        AGENT_CIRCUIT_BREAKER_ADDRESS, IAccountKeychain,
        IAccountKeychain::{CallScope, KeyRestrictions, SelectorRule, SignatureType, TokenLimit},
        ISciAgentState::tripKeyCall,
        authorizeKeyCall, getRemainingLimitWithPeriodCall,
    },
    predeploys::{IERC20, ISCI20, ISCIAgentDelegator, SCI_AGENT_DELEGATOR_ADDRESS},
};

type TestEvm = BaseEvm<InMemoryDB, NoOpInspector, alloy_evm::precompiles::PrecompilesMap>;

/// Test fixture: a fresh BaseEvm with three funded EOAs and the SCI precompiles wired.
struct AgentFixture {
    evm: TestEvm,
    /// Funded EOA used as the "root" account.
    pub root: Address,
    /// Funded EOA used as the "session key" account.
    pub session_key: Address,
    /// Another funded EOA available for test scenarios that need a third party.
    pub bystander: Address,
    /// Per-account nonce so multi-tx tests don't trip `NonceTooLow`.
    nonces: std::collections::HashMap<Address, u64>,
}

impl AgentFixture {
    fn new() -> Self {
        let root = address!("0x000000000000000000000000000000000000C001");
        let session_key = address!("0x000000000000000000000000000000000000C002");
        let bystander = address!("0x000000000000000000000000000000000000C003");

        let mut db = InMemoryDB::default();
        let one_eth_x_100 = U256::from(10u128).pow(U256::from(20u64));
        for addr in [root, session_key, bystander, AGENT_CIRCUIT_BREAKER_ADDRESS] {
            db.insert_account_info(
                addr,
                AccountInfo {
                    balance: one_eth_x_100,
                    ..Default::default()
                },
            );
        }

        let cfg = CfgEnv::new_with_spec(BaseSpecId::new(BaseUpgrade::Isthmus)).with_chain_id(42001);
        let mut block = BlockEnv::default();
        block.basefee = 0; // no basefee in tests; we still pay gas_price
        let env = EvmEnv {
            cfg_env: cfg,
            block_env: block,
        };
        let evm = BaseEvmFactory::default().create_evm(db, env);
        Self {
            evm,
            root,
            session_key,
            bystander,
            nonces: std::collections::HashMap::new(),
        }
    }

    /// Returns the next nonce for `caller` and increments the local counter.
    fn next_nonce(&mut self, caller: Address) -> u64 {
        let n = self.nonces.entry(caller).or_insert(0);
        let cur = *n;
        *n += 1;
        cur
    }

    /// Runs a tx and commits the resulting state. Returns the `ExecutionResult`.
    fn run_tx(&mut self, tx: BaseTransaction<TxEnv>) -> Result<ExecutionResult<BaseHaltReason>, String> {
        let outcome = self.evm.transact_raw(tx).map_err(|e| format!("{e:?}"))?;
        let state = outcome.state;
        let result = outcome.result;
        // Persist to the underlying InMemoryDB so subsequent txs see the change.
        self.evm.ctx_mut().journaled_state.database.commit(state);
        Ok(result)
    }

    /// Authorizes `key_id` on the keychain for `account` (the root).
    fn authorize_key(
        &mut self,
        account: Address,
        key_id: Address,
        limits: Vec<TokenLimit>,
        allow_any_calls: bool,
        allowed_calls: Vec<CallScope>,
    ) {
        let enforce = !limits.is_empty();
        let calldata = authorizeKeyCall {
            keyId: key_id,
            signatureType: SignatureType::Secp256k1,
            config: KeyRestrictions {
                expiry: u64::MAX,
                enforceLimits: enforce,
                limits,
                allowAnyCalls: allow_any_calls,
                allowedCalls: allowed_calls,
            },
        }
        .abi_encode();
        let nonce = self.next_nonce(account);
        let tx = BaseTransaction::builder()
            .base(
                TxEnv::builder()
                    .caller(account)
                    .kind(TxKind::Call(ACCOUNT_KEYCHAIN_ADDRESS))
                    .data(Bytes::from(calldata))
                    .nonce(nonce)
                    .gas_limit(5_000_000)
                    .gas_price(1)
                    .chain_id(Some(42001)),
            )
            .build_fill();
        let result = self.run_tx(tx).expect("authorize_key tx failed");
        assert!(result.is_success(), "authorize_key reverted: {result:?}");
    }

    /// Deploys `bytecode` to `addr` (overwrites any existing code).
    fn deploy_raw_code(&mut self, addr: Address, raw_bytes: Vec<u8>) {
        let bytecode = Bytecode::new_raw(Bytes::from(raw_bytes));
        let code_hash = bytecode.hash_slow();
        let db = &mut self.evm.ctx_mut().journaled_state.database;
        let mut info = db.basic(addr).expect("db basic read").unwrap_or_default();
        info.code_hash = code_hash;
        info.code = Some(bytecode);
        db.insert_account_info(addr, info);
    }

    /// Writes EIP-7702 delegation bytecode `0xef0100 || delegate` to `eoa`.
    fn set_7702_delegation(&mut self, eoa: Address, delegate: Address) {
        let mut code_bytes = vec![0xef, 0x01, 0x00];
        code_bytes.extend_from_slice(delegate.as_slice());
        let bytecode = Bytecode::new_raw(Bytes::from(code_bytes));
        let code_hash = bytecode.hash_slow();

        // Pull the account, set code + hash, push back.
        let db = &mut self.evm.ctx_mut().journaled_state.database;
        let mut info = db
            .basic(eoa)
            .expect("db basic read")
            .unwrap_or_default();
        info.code_hash = code_hash;
        info.code = Some(bytecode);
        db.insert_account_info(eoa, info);
    }

    /// Trips a session key via the SciAgentState precompile.
    fn trip_key(&mut self, session_key: Address) {
        let calldata = tripKeyCall {
            sessionKey: session_key,
        }
        .abi_encode();
        let nonce = self.next_nonce(AGENT_CIRCUIT_BREAKER_ADDRESS);
        let tx = BaseTransaction::builder()
            .base(
                TxEnv::builder()
                    .caller(AGENT_CIRCUIT_BREAKER_ADDRESS)
                    .kind(TxKind::Call(SCI_AGENT_STATE_ADDRESS))
                    .data(Bytes::from(calldata))
                    .nonce(nonce)
                    .gas_limit(5_000_000)
                    .gas_price(1)
                    .chain_id(Some(42001)),
            )
            .build_fill();
        let result = self.run_tx(tx).expect("trip_key tx failed");
        assert!(result.is_success(), "trip_key reverted: {result:?}");
    }

    /// Sends an agent tx: from = session_key, to = root, input = execute(calls).
    fn send_agent_tx(
        &mut self,
        calls: Vec<ISCIAgentDelegator::Call>,
    ) -> Result<ExecutionResult<BaseHaltReason>, String> {
        let calldata = ISCIAgentDelegator::executeCall { calls }.abi_encode();
        let session_key = self.session_key;
        let root = self.root;
        let nonce = self.next_nonce(session_key);
        let tx = BaseTransaction::builder()
            .base(
                TxEnv::builder()
                    .caller(session_key)
                    .kind(TxKind::Call(root))
                    .data(Bytes::from(calldata))
                    .nonce(nonce)
                    .gas_limit(5_000_000)
                    .gas_price(1)
                    .chain_id(Some(42001)),
            )
            .build_fill();
        self.run_tx(tx)
    }

    /// Reads the remaining spending limit for `(account, key_id, token)`.
    fn remaining_limit(&mut self, account: Address, key_id: Address, token: Address) -> U256 {
        let calldata = getRemainingLimitWithPeriodCall {
            account,
            keyId: key_id,
            token,
        }
        .abi_encode();
        let bystander = self.bystander;
        let nonce = self.next_nonce(bystander);
        // Use bystander as caller so this read query doesn't clash with `account`'s nonce.
        let tx = BaseTransaction::builder()
            .base(
                TxEnv::builder()
                    .caller(bystander)
                    .kind(TxKind::Call(ACCOUNT_KEYCHAIN_ADDRESS))
                    .data(Bytes::from(calldata))
                    .nonce(nonce)
                    .gas_limit(5_000_000)
                    .gas_price(1)
                    .chain_id(Some(42001)),
            )
            .build_fill();
        let result = self.run_tx(tx).expect("remaining_limit tx failed");
        match result {
            ExecutionResult::Success { output: Output::Call(b), .. } => {
                let decoded = getRemainingLimitWithPeriodCall::abi_decode_returns(&b)
                    .expect("decode getRemainingLimitWithPeriod");
                decoded.remaining
            }
            other => panic!("remaining_limit unexpected result: {other:?}"),
        }
    }
}

// ====================================================================================
// Sanity test — fixture wires up correctly
// ====================================================================================

/// Builds a no-op inner Call (zero value, empty data, harmless target).
fn noop_call(target: Address) -> ISCIAgentDelegator::Call {
    ISCIAgentDelegator::Call {
        target,
        value: U256::ZERO,
        data: Bytes::new(),
    }
}

#[test]
fn fixture_smoke_plain_tx_works() {
    let mut fx = AgentFixture::new();
    let nonce = fx.next_nonce(fx.root);
    let tx = BaseTransaction::builder()
        .base(
            TxEnv::builder()
                .caller(fx.root)
                .kind(TxKind::Call(fx.bystander))
                .value(U256::from(1u64))
                .nonce(nonce)
                .gas_limit(50_000)
                .gas_price(1)
                .chain_id(Some(42001)),
        )
        .build_fill();
    let result = fx.run_tx(tx).expect("plain transfer should not fail");
    assert!(result.is_success(), "plain transfer must succeed: {result:?}");
}

// ====================================================================================
// N. No-op hook tests — hook stays out of the way for non-agent traffic.
// ====================================================================================

#[test]
fn no_7702_no_hook() {
    // session_key sends an `execute(Call[])` tx to root, but root has no 7702 code.
    // Hook should treat it as a plain tx: no scope check, no quota deduction, just a
    // normal EVM call to an EOA (which is a no-op since EOAs have no code).
    let mut fx = AgentFixture::new();
    let result = fx
        .send_agent_tx(vec![noop_call(fx.bystander)])
        .expect("plain tx should not be rejected");
    assert!(
        result.is_success(),
        "tx without 7702 must pass through hook unchanged: {result:?}",
    );
}

#[test]
fn wrong_delegate_no_hook() {
    // root has 7702 delegation to some random address, NOT SCI_AGENT_DELEGATOR_ADDRESS.
    // Hook should not detect this as an agent tx.
    let mut fx = AgentFixture::new();
    let wrong_delegate = address!("0x0000000000000000000000000000000000DEADBE");
    fx.set_7702_delegation(fx.root, wrong_delegate);
    let result = fx
        .send_agent_tx(vec![noop_call(fx.bystander)])
        .expect("plain tx should not be rejected");
    assert!(
        result.is_success(),
        "tx with wrong 7702 delegate must pass through hook: {result:?}",
    );
}

// ====================================================================================
// O. Agent-tx happy path tests — hook fires, deducts quota, tx succeeds.
// ====================================================================================

const TEST_TOKEN: Address = address!("0x000000000000000000000000000000000000ABCD");

/// Sets up the canonical "agent ready to act" state:
/// - 7702 delegation root → SCI_AGENT_DELEGATOR_ADDRESS
/// - Session key authorized on keychain with a 1000-unit limit on `TEST_TOKEN`
/// - `allow_any_calls = true` so scope checks are no-ops (we test scope separately).
fn authorize_unscoped_with_limit(fx: &mut AgentFixture, limit: U256) {
    fx.authorize_key(
        fx.root,
        fx.session_key,
        vec![TokenLimit {
            token: TEST_TOKEN,
            amount: limit,
            period: 0,
        }],
        true,
        vec![],
    );
    fx.set_7702_delegation(fx.root, SCI_AGENT_DELEGATOR_ADDRESS);
}

#[test]
fn agent_tx_with_authorized_key_succeeds_and_deducts_limit() {
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));

    let pre = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(pre, U256::from(1000u64));

    let transfer_data = IERC20::transferCall {
        to: fx.bystander,
        amount: U256::from(300u64),
    }
    .abi_encode();
    let result = fx
        .send_agent_tx(vec![ISCIAgentDelegator::Call {
            target: TEST_TOKEN,
            value: U256::ZERO,
            data: Bytes::from(transfer_data),
        }])
        .expect("happy path agent tx should succeed");
    assert!(result.is_success(), "agent tx must succeed: {result:?}");

    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(post, U256::from(700u64), "transfer(300) must deduct 300");
}

#[test]
fn approve_pessimistic_deduct() {
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));

    let approve_data = IERC20::approveCall {
        spender: fx.bystander,
        amount: U256::from(600u64),
    }
    .abi_encode();
    let result = fx
        .send_agent_tx(vec![ISCIAgentDelegator::Call {
            target: TEST_TOKEN,
            value: U256::ZERO,
            data: Bytes::from(approve_data),
        }])
        .expect("approve tx should succeed");
    assert!(result.is_success(), "approve must succeed: {result:?}");

    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(
        post,
        U256::from(400u64),
        "approve(600) must pessimistically deduct 600 (no refund of unused allowance)",
    );
}

#[test]
fn transfer_from_doesnt_deduct() {
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));

    let tf_data = IERC20::transferFromCall {
        from: fx.bystander,
        to: fx.root,
        amount: U256::from(500u64),
    }
    .abi_encode();
    let result = fx
        .send_agent_tx(vec![ISCIAgentDelegator::Call {
            target: TEST_TOKEN,
            value: U256::ZERO,
            data: Bytes::from(tf_data),
        }])
        .expect("transferFrom tx should succeed");
    assert!(result.is_success(), "transferFrom must succeed: {result:?}");

    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(
        post,
        U256::from(1000u64),
        "transferFrom must NOT deduct quota (spender ≠ session key root)",
    );
}

#[test]
fn transfer_with_memo_deducts_amount() {
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));

    let twm_data = ISCI20::transferWithMemoCall {
        to: fx.bystander,
        amount: U256::from(250u64),
        memo: B256::repeat_byte(0xab),
    }
    .abi_encode();
    let result = fx
        .send_agent_tx(vec![ISCIAgentDelegator::Call {
            target: TEST_TOKEN,
            value: U256::ZERO,
            data: Bytes::from(twm_data),
        }])
        .expect("transferWithMemo tx should succeed");
    assert!(result.is_success(), "transferWithMemo must succeed: {result:?}");

    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(post, U256::from(750u64), "transferWithMemo(250) must deduct 250");
}

// ====================================================================================
// P. Agent-tx rejection tests — hook detects violations and rejects the entire tx.
// ====================================================================================

#[test]
fn scope_violation_rejects_tx() {
    // Authorize key with a scope that ONLY permits transfer() on TEST_TOKEN.
    // Sending a tx with approve() on TEST_TOKEN should fail scope check.
    let mut fx = AgentFixture::new();
    let allowed_calls = vec![CallScope {
        target: TEST_TOKEN,
        selectorRules: vec![SelectorRule {
            selector: IERC20::transferCall::SELECTOR.into(),
            recipients: vec![],
        }],
    }];
    fx.authorize_key(fx.root, fx.session_key, vec![], false, allowed_calls);
    fx.set_7702_delegation(fx.root, SCI_AGENT_DELEGATOR_ADDRESS);

    // approve is not in the allowed selector set → scope violation.
    let approve_data = IERC20::approveCall {
        spender: fx.bystander,
        amount: U256::from(100u64),
    }
    .abi_encode();
    let result = fx.send_agent_tx(vec![ISCIAgentDelegator::Call {
        target: TEST_TOKEN,
        value: U256::ZERO,
        data: Bytes::from(approve_data),
    }]);

    assert!(
        result.is_err(),
        "scope violation must reject the tx, got: {result:?}",
    );
    let err = result.unwrap_err();
    assert!(
        err.contains("SCI hook"),
        "expected SCI hook rejection, got: {err}",
    );
}

#[test]
fn quota_exhaustion_rejects_tx() {
    // Authorize key with limit=100. Transferring 200 must fail.
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(100u64));

    let transfer_data = IERC20::transferCall {
        to: fx.bystander,
        amount: U256::from(200u64),
    }
    .abi_encode();
    let result = fx.send_agent_tx(vec![ISCIAgentDelegator::Call {
        target: TEST_TOKEN,
        value: U256::ZERO,
        data: Bytes::from(transfer_data),
    }]);

    assert!(
        result.is_err(),
        "quota exhaustion must reject the tx, got: {result:?}",
    );
}

#[test]
fn tripped_key_rejects_tx() {
    // Authorize, then trip the session key via SciAgentState.
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));
    fx.trip_key(fx.session_key);

    let transfer_data = IERC20::transferCall {
        to: fx.bystander,
        amount: U256::from(50u64),
    }
    .abi_encode();
    let result = fx.send_agent_tx(vec![ISCIAgentDelegator::Call {
        target: TEST_TOKEN,
        value: U256::ZERO,
        data: Bytes::from(transfer_data),
    }]);

    assert!(
        result.is_err(),
        "tripped key must reject the tx, got: {result:?}",
    );
}

// ====================================================================================
// Q. Rollback + skip-path tests — journal hygiene + deposit/CREATE bypass.
// ====================================================================================

#[test]
fn batch_partial_failure_rolls_back_deductions() {
    // Hook deducts call 1 (800), then call 2 (300) exceeds remaining quota (200) →
    // hook rejects entire batch → checkpoint_revert rolls back the 800.
    // Verifies the hook's all-or-nothing checkpoint semantics for multi-call batches.
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));

    let call_1 = ISCIAgentDelegator::Call {
        target: TEST_TOKEN,
        value: U256::ZERO,
        data: Bytes::from(
            IERC20::transferCall {
                to: fx.bystander,
                amount: U256::from(800u64),
            }
            .abi_encode(),
        ),
    };
    let call_2 = ISCIAgentDelegator::Call {
        target: TEST_TOKEN,
        value: U256::ZERO,
        data: Bytes::from(
            IERC20::transferCall {
                to: fx.bystander,
                amount: U256::from(300u64),
            }
            .abi_encode(),
        ),
    };
    let result = fx.send_agent_tx(vec![call_1, call_2]);
    assert!(
        result.is_err(),
        "partial batch failure must reject the tx, got: {result:?}",
    );

    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(
        post,
        U256::from(1000u64),
        "call 1's 800-unit deduction must be rolled back when call 2 fails",
    );
}

#[test]
fn body_revert_rolls_back_deduction_strong_r1() {
    // Q4 strong-R1: the hook authorizes the spend (passes scope + pre-flight check
    // against quota) but the EVM body reverts. With strong R1 the deduction is
    // deferred to `execution_result`, which only fires on the success path — so a
    // body revert leaves quota untouched.
    //
    // Implementation: pre-execution hook does read-only pre-flight check; post-
    // execution `SciHandler::execution_result` invokes `apply_post_execution_deductions`
    // only when `frame_result.interpreter_result().result.is_ok()`.
    //
    // Setup: deploy "always revert" bytecode at SCI_AGENT_DELEGATOR_ADDRESS so the
    // 7702-routed execution from `root` immediately reverts.
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));
    // `60 00 60 00 fd` = PUSH1 0; PUSH1 0; REVERT
    fx.deploy_raw_code(SCI_AGENT_DELEGATOR_ADDRESS, vec![0x60, 0x00, 0x60, 0x00, 0xfd]);

    let transfer_data = IERC20::transferCall {
        to: fx.bystander,
        amount: U256::from(100u64),
    }
    .abi_encode();
    let result = fx
        .send_agent_tx(vec![ISCIAgentDelegator::Call {
            target: TEST_TOKEN,
            value: U256::ZERO,
            data: Bytes::from(transfer_data),
        }])
        .expect("body-revert tx should complete (not reject)");
    assert!(
        matches!(result, ExecutionResult::Revert { .. }),
        "EVM body must revert, got: {result:?}",
    );

    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(
        post,
        U256::from(1000u64),
        "strong R1: body revert must leave quota untouched (deferred deduction never applied)",
    );
}

#[test]
fn create_tx_skips_hook() {
    // A CREATE tx (no `to`) can't be 7702-delegated. Hook must short-circuit and
    // never enter the agent-tx path, even if root is 7702-delegated and key is
    // authorized.
    let mut fx = AgentFixture::new();
    authorize_unscoped_with_limit(&mut fx, U256::from(1000u64));

    // Send a CREATE tx from session_key with bytecode `60 00` (PUSH1 0; STOP).
    let nonce = fx.next_nonce(fx.session_key);
    let tx = BaseTransaction::builder()
        .base(
            TxEnv::builder()
                .caller(fx.session_key)
                .kind(TxKind::Create)
                .data(Bytes::from(vec![0x60, 0x00, 0x00]))
                .nonce(nonce)
                .gas_limit(500_000)
                .gas_price(1)
                .chain_id(Some(42001)),
        )
        .build_fill();
    let result = fx.run_tx(tx).expect("CREATE tx should not be hook-rejected");
    assert!(result.is_success(), "CREATE must succeed, got: {result:?}");

    // Quota untouched — hook never ran.
    let post = fx.remaining_limit(fx.root, fx.session_key, TEST_TOKEN);
    assert_eq!(post, U256::from(1000u64), "CREATE tx must not affect quota");
}

#[test]
fn correct_7702_but_no_key_no_hook() {
    // root has 7702 → SCI_AGENT_DELEGATOR_ADDRESS, but no session key authorized in
    // keychain. Hook should detect 7702 but fail the key probe → pass through.
    let mut fx = AgentFixture::new();
    fx.set_7702_delegation(fx.root, SCI_AGENT_DELEGATOR_ADDRESS);
    let result = fx
        .send_agent_tx(vec![noop_call(fx.bystander)])
        .expect("plain tx should not be rejected");
    assert!(
        result.is_success(),
        "tx with 7702 but no registered key must pass through hook: {result:?}",
    );
}
