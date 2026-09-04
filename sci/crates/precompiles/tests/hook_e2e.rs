//! Integration tests for the SCI Chain AA pre-execution hook (Plan A, type `0x76`).
//!
//! These drive [`base_common_evm::SciHandler`] through a real `BaseEvm` backed by
//! `InMemoryDB` to verify behaviors that unit tests against `HashMapStorageProvider`
//! can't reach:
//!
//! - the AA batch executor + keychain hook interplay (`aa_parts` → `run_aa_keychain_hook`)
//! - Q4 strong-R1: deferred deductions apply only on success; a body-reverting batch
//!   costs no token quota…
//! - …but DOES cost gas quota when `fee_payer == root` sponsors gas (review finding M-1)
//! - `transferFrom(from == root, …)` counts against the per-token quota (finding M-2)
//! - batch atomicity (one failing call reverts the whole batch)
//! - circuit-breaker and scope rejections surface as tx-level errors (no quota burn)
//!
//! Referenced by CLAUDE.md's "Upstream Tempo Sync" step 6 verify command:
//! `cargo test -p sci-precompiles --test hook_e2e`.

use alloy_evm::{Evm, EvmEnv, EvmFactory};
use alloy_primitives::{Address, Bytes, TxKind, U256, address};
use alloy_sol_types::SolCall;
use base_common_chains::BaseUpgrade;
use base_common_consensus::Call;
use base_common_evm::{
    AaTransactionParts, BaseEvm, BaseEvmFactory, BaseHaltReason, BaseSpecId, BaseTransaction,
};
use revm::{
    DatabaseCommit,
    bytecode::Bytecode,
    context::{BlockEnv, CfgEnv, TxEnv},
    context_interface::result::{ExecutionResult, Output},
    database::InMemoryDB,
    database_interface::Database,
    inspector::NoOpInspector,
    state::AccountInfo,
};
use sci_precompiles::{ACCOUNT_KEYCHAIN_ADDRESS, SCI_AGENT_STATE_ADDRESS};
use tempo_contracts::{
    precompiles::{
        AGENT_CIRCUIT_BREAKER_ADDRESS,
        IAccountKeychain::{CallScope, KeyRestrictions, SignatureType, TokenLimit},
        ISciAgentState::tripKeyCall,
        authorizeKeyCall, getRemainingLimitWithPeriodCall,
    },
    predeploys::IERC20,
};

type TestEvm = BaseEvm<InMemoryDB, NoOpInspector, alloy_evm::precompiles::PrecompilesMap>;

const CHAIN_ID: u64 = 42001;
/// EVM bytecode that unconditionally reverts: PUSH1 0, PUSH1 0, REVERT.
const REVERT_BYTECODE: [u8; 5] = [0x60, 0x00, 0x60, 0x00, 0xfd];

/// Test fixture: a fresh `BaseEvm` with funded EOAs and the SCI precompiles wired.
struct AgentFixture {
    evm: TestEvm,
    /// Funded EOA used as the "root" account.
    pub root: Address,
    /// Funded EOA used as the "session key" (AA tx signer).
    pub session_key: Address,
    /// Another funded EOA for third-party scenarios and read queries.
    pub bystander: Address,
    /// An address holding empty code — calls to it succeed as no-ops, so ERC-20-shaped
    /// calldata sent at it exercises the hook's metering without a real token.
    pub token: Address,
    /// An address holding [`REVERT_BYTECODE`] — any call to it reverts.
    pub reverter: Address,
    /// Per-account nonce so multi-tx tests don't trip `NonceTooLow`.
    nonces: std::collections::HashMap<Address, u64>,
}

impl AgentFixture {
    fn new() -> Self {
        let root = address!("0x000000000000000000000000000000000000C001");
        let session_key = address!("0x000000000000000000000000000000000000C002");
        let bystander = address!("0x000000000000000000000000000000000000C003");
        let token = address!("0x000000000000000000000000000000000000C004");
        let reverter = address!("0x000000000000000000000000000000000000C005");

        let mut db = InMemoryDB::default();
        let one_eth_x_100 = U256::from(10u128).pow(U256::from(20u64));
        for addr in [root, session_key, bystander, AGENT_CIRCUIT_BREAKER_ADDRESS] {
            db.insert_account_info(
                addr,
                AccountInfo { balance: one_eth_x_100, ..Default::default() },
            );
        }

        let cfg =
            CfgEnv::new_with_spec(BaseSpecId::new(BaseUpgrade::Isthmus)).with_chain_id(CHAIN_ID);
        let block = BlockEnv { basefee: 0, ..Default::default() };
        let env = EvmEnv { cfg_env: cfg, block_env: block };
        let evm = BaseEvmFactory::default().create_evm(db, env);

        let mut fx = Self {
            evm,
            root,
            session_key,
            bystander,
            token,
            reverter,
            nonces: std::collections::HashMap::new(),
        };
        fx.deploy_raw_code(reverter, REVERT_BYTECODE.to_vec());
        fx
    }

    /// Returns the next nonce for `caller` and increments the local counter.
    fn next_nonce(&mut self, caller: Address) -> u64 {
        let n = self.nonces.entry(caller).or_insert(0);
        let cur = *n;
        *n += 1;
        cur
    }

    /// Runs a tx and commits the resulting state. Returns the `ExecutionResult`.
    fn run_tx(
        &mut self,
        tx: BaseTransaction<TxEnv>,
    ) -> Result<ExecutionResult<BaseHaltReason>, String> {
        let outcome = self.evm.transact_raw(tx).map_err(|e| format!("{e:?}"))?;
        // Persist to the underlying InMemoryDB so subsequent txs see the change.
        self.evm.ctx_mut().journaled_state.database.commit(outcome.state);
        Ok(outcome.result)
    }

    /// Builds and runs an AA (`0x76`) agent tx: signer = `session_key`, executing `calls`
    /// on behalf of `root` (when set), with optional sponsored gas.
    fn send_aa_tx(
        &mut self,
        root: Option<Address>,
        fee_payer: Option<Address>,
        calls: Vec<Call>,
    ) -> Result<ExecutionResult<BaseHaltReason>, String> {
        let session_key = self.session_key;
        let nonce = self.next_nonce(session_key);
        // The base env mirrors the consensus conversion (`to_eip1559_first_call` with
        // `value = 0`); `SciHandler` executes the real batch from `aa_parts`.
        let first = calls.first().expect("non-empty batch");
        let mut tx = BaseTransaction::builder()
            .base(
                TxEnv::builder()
                    .caller(session_key)
                    .kind(first.to)
                    .data(first.input.clone())
                    .nonce(nonce)
                    .gas_limit(5_000_000)
                    .gas_price(1)
                    .chain_id(Some(CHAIN_ID)),
            )
            .build_fill();
        tx.aa = Some(AaTransactionParts { calls, root, fee_payer });
        let result = self.run_tx(tx);
        if result.is_err() {
            // A hook-rejected tx discards its journal — including the nonce bump — so
            // the local counter must roll back to stay in sync with state.
            *self.nonces.get_mut(&session_key).expect("counter exists") -= 1;
        }
        result
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
                    .chain_id(Some(CHAIN_ID)),
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

    /// Trips a session key via the `SciAgentState` precompile (caller must be the
    /// `AgentCircuitBreaker` predeploy address — mirrored here directly).
    fn trip_key(&mut self, session_key: Address) {
        let calldata = tripKeyCall { sessionKey: session_key }.abi_encode();
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
                    .chain_id(Some(CHAIN_ID)),
            )
            .build_fill();
        let result = self.run_tx(tx).expect("trip_key tx failed");
        assert!(result.is_success(), "trip_key reverted: {result:?}");
    }

    /// Reads the remaining spending limit for `(account, key_id, token)`.
    fn remaining_limit(&mut self, account: Address, key_id: Address, token: Address) -> U256 {
        let calldata =
            getRemainingLimitWithPeriodCall { account, keyId: key_id, token }.abi_encode();
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
                    .chain_id(Some(CHAIN_ID)),
            )
            .build_fill();
        let result = self.run_tx(tx).expect("remaining_limit tx failed");
        match result {
            ExecutionResult::Success { output: Output::Call(b), .. } => {
                getRemainingLimitWithPeriodCall::abi_decode_returns(&b)
                    .expect("decode getRemainingLimitWithPeriod")
                    .remaining
            }
            other => panic!("remaining_limit unexpected result: {other:?}"),
        }
    }

    /// Reads an account's current balance from the committed DB.
    fn balance(&mut self, addr: Address) -> U256 {
        let db = &mut self.evm.ctx_mut().journaled_state.database;
        db.basic(addr).expect("db basic read").map(|i| i.balance).unwrap_or_default()
    }
}

/// A `TokenLimit` with no period.
fn limit(token: Address, amount: u64) -> TokenLimit {
    TokenLimit { token, amount: U256::from(amount), period: 0 }
}

/// An inner AA call.
fn call(to: Address, value: u64, input: Vec<u8>) -> Call {
    Call { to: TxKind::Call(to), value: U256::from(value), input: Bytes::from(input) }
}

/// `ERC20.transfer(to, amount)` calldata.
fn transfer_data(to: Address, amount: u64) -> Vec<u8> {
    IERC20::transferCall { to, amount: U256::from(amount) }.abi_encode()
}

/// `ERC20.transferFrom(from, to, amount)` calldata.
fn transfer_from_data(from: Address, to: Address, amount: u64) -> Vec<u8> {
    IERC20::transferFromCall { from, to, amount: U256::from(amount) }.abi_encode()
}

// ====================================================================================
// Sanity / non-agent traffic
// ====================================================================================

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
                .chain_id(Some(CHAIN_ID)),
        )
        .build_fill();
    let result = fx.run_tx(tx).expect("plain transfer should not fail");
    assert!(result.is_success(), "plain transfer must succeed: {result:?}");
}

/// An AA tx without `root` is a plain batch executed as the signer: no keychain gate,
/// no authorization required.
#[test]
fn aa_without_root_executes_as_signer_without_keychain() {
    let mut fx = AgentFixture::new();
    let bystander = fx.bystander;
    let before = fx.balance(bystander);
    let result = fx
        .send_aa_tx(None, None, vec![call(bystander, 7, vec![])])
        .expect("root-less AA tx must not hit the keychain gate");
    assert!(result.is_success(), "{result:?}");
    assert_eq!(fx.balance(bystander), before + U256::from(7u64), "value moved from signer");
}

// ====================================================================================
// Authorization gate
// ====================================================================================

/// An AA tx whose `root` never authorized the signing session key is rejected at the
/// hook — before any call executes.
#[test]
fn unauthorized_session_key_rejected() {
    let mut fx = AgentFixture::new();
    let (root, bystander) = (fx.root, fx.bystander);
    let err = fx
        .send_aa_tx(Some(root), None, vec![call(bystander, 1, vec![])])
        .expect_err("AA tx without keychain authorization must be rejected");
    assert!(err.contains("unauthorized"), "unexpected error: {err}");
}

/// A tripped session key's AA tx is rejected by the circuit breaker.
#[test]
fn circuit_breaker_blocks_tripped_key() {
    let mut fx = AgentFixture::new();
    let (root, session_key, bystander) = (fx.root, fx.session_key, fx.bystander);
    fx.authorize_key(root, session_key, vec![], true, vec![]);
    fx.trip_key(session_key);

    let err = fx
        .send_aa_tx(Some(root), None, vec![call(bystander, 1, vec![])])
        .expect_err("tripped session key must be rejected");
    assert!(err.contains("rejected"), "unexpected error: {err}");
}

/// A scoped key in deny-all mode (`allowAnyCalls = false`, empty `allowedCalls`)
/// rejects every call.
#[test]
fn deny_all_scope_rejects_batch() {
    let mut fx = AgentFixture::new();
    let (root, session_key, bystander) = (fx.root, fx.session_key, fx.bystander);
    fx.authorize_key(root, session_key, vec![], false, vec![]);

    let err = fx
        .send_aa_tx(Some(root), None, vec![call(bystander, 0, vec![])])
        .expect_err("deny-all scope must reject the batch");
    assert!(err.contains("rejected"), "unexpected error: {err}");
}

// ====================================================================================
// Q4 strong-R1 spending-limit semantics + D-gas (M-1)
// ====================================================================================

/// Happy path: a successful in-limit ERC-20 transfer deducts exactly its amount from
/// the per-token quota.
#[test]
fn successful_batch_deducts_quota() {
    let mut fx = AgentFixture::new();
    let (root, session_key, token, bystander) = (fx.root, fx.session_key, fx.token, fx.bystander);
    fx.authorize_key(root, session_key, vec![limit(token, 1_000)], true, vec![]);
    assert_eq!(fx.remaining_limit(root, session_key, token), U256::from(1_000u64));

    let result = fx
        .send_aa_tx(Some(root), None, vec![call(token, 0, transfer_data(bystander, 400))])
        .expect("in-limit transfer must pass the hook");
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        fx.remaining_limit(root, session_key, token),
        U256::from(600u64),
        "quota deducted by the transferred amount"
    );
}

/// A batch total exceeding the per-token quota is rejected in the hook pre-flight —
/// before execution, with no deduction.
#[test]
fn over_limit_batch_rejected_without_deduction() {
    let mut fx = AgentFixture::new();
    let (root, session_key, token, bystander) = (fx.root, fx.session_key, fx.token, fx.bystander);
    fx.authorize_key(root, session_key, vec![limit(token, 100)], true, vec![]);

    let err = fx
        .send_aa_tx(Some(root), None, vec![call(token, 0, transfer_data(bystander, 150))])
        .expect_err("over-limit transfer must be rejected");
    assert!(err.contains("rejected"), "unexpected error: {err}");
    assert_eq!(
        fx.remaining_limit(root, session_key, token),
        U256::from(100u64),
        "hook rejection must not deduct quota"
    );
}

/// Q4 strong-R1: the hook passes (pre-flight fits), the body then reverts — the token
/// quota must be untouched (deduction is deferred to success). Signer pays gas here,
/// so no gas metering either.
#[test]
fn body_revert_rolls_back_deduction_strong_r1() {
    let mut fx = AgentFixture::new();
    let (root, session_key, reverter, bystander) =
        (fx.root, fx.session_key, fx.reverter, fx.bystander);
    fx.authorize_key(root, session_key, vec![limit(reverter, 1_000)], true, vec![]);

    let result = fx
        .send_aa_tx(Some(root), None, vec![call(reverter, 0, transfer_data(bystander, 500))])
        .expect("hook passes; the revert happens in the body");
    assert!(!result.is_success(), "call to the reverter must revert: {result:?}");
    assert_eq!(
        fx.remaining_limit(root, session_key, reverter),
        U256::from(1_000u64),
        "body revert must not cost token quota (strong-R1)"
    );
}

/// Review finding M-1: when `fee_payer == root` sponsors gas, a body-reverting batch
/// still burns root's real ETH for gas — so the `address(0)` sentinel quota MUST be
/// charged even on revert, or a session key could drain root with deliberately
/// reverting batches at zero quota cost.
#[test]
fn gas_quota_charged_on_revert_with_sponsored_gas() {
    let mut fx = AgentFixture::new();
    let (root, session_key, reverter) = (fx.root, fx.session_key, fx.reverter);
    let sentinel = Address::ZERO;
    let quota = 1_000_000_000u64;
    fx.authorize_key(root, session_key, vec![limit(sentinel, quota)], true, vec![]);

    let result = fx
        .send_aa_tx(Some(root), Some(root), vec![call(reverter, 0, vec![])])
        .expect("hook passes; the revert happens in the body");
    assert!(!result.is_success(), "call to the reverter must revert: {result:?}");

    let remaining = fx.remaining_limit(root, session_key, sentinel);
    let expected_gas_charge = U256::from(result.gas_used()); // max_fee_per_gas == 1
    assert_eq!(
        U256::from(quota) - remaining,
        expected_gas_charge,
        "reverting sponsored batch must charge gas_used * max_fee against the sentinel"
    );
    assert!(!expected_gas_charge.is_zero(), "a reverting batch always burns some gas");
}

/// The success-path counterpart: a sponsored successful batch charges gas + value
/// against the sentinel.
#[test]
fn gas_quota_charged_on_success_with_sponsored_gas() {
    let mut fx = AgentFixture::new();
    let (root, session_key, bystander) = (fx.root, fx.session_key, fx.bystander);
    let sentinel = Address::ZERO;
    let quota = 1_000_000_000u64;
    fx.authorize_key(root, session_key, vec![limit(sentinel, quota)], true, vec![]);

    let result = fx
        .send_aa_tx(Some(root), Some(root), vec![call(bystander, 25, vec![])])
        .expect("in-quota sponsored batch must pass");
    assert!(result.is_success(), "{result:?}");

    let remaining = fx.remaining_limit(root, session_key, sentinel);
    let expected = U256::from(result.gas_used()) + U256::from(25u64); // gas + native value
    assert_eq!(U256::from(quota) - remaining, expected);
}

// ====================================================================================
// Batch atomicity
// ====================================================================================

/// One failing call reverts the whole batch: state changes from earlier calls are
/// rolled back and no token quota is deducted.
#[test]
fn batch_partial_failure_rolls_back_whole_batch() {
    let mut fx = AgentFixture::new();
    let (root, session_key, token, bystander, reverter) =
        (fx.root, fx.session_key, fx.token, fx.bystander, fx.reverter);
    let sentinel = Address::ZERO;
    // The native value in call #1 meters against the address(0) sentinel, and an
    // enforce_limits key with no sentinel row would be pre-flight-rejected — so grant one.
    fx.authorize_key(
        root,
        session_key,
        vec![limit(token, 1_000), limit(sentinel, 1_000_000)],
        true,
        vec![],
    );
    let bystander_before = fx.balance(bystander);

    let result = fx
        .send_aa_tx(
            Some(root),
            None,
            vec![
                call(bystander, 9, vec![]),                    // would succeed alone
                call(token, 0, transfer_data(bystander, 300)), // no-op target, fine
                call(reverter, 0, vec![]),                     // fails → batch reverts
            ],
        )
        .expect("hook passes; the revert happens in the body");
    assert!(!result.is_success(), "batch with a failing call must revert: {result:?}");
    assert_eq!(
        fx.balance(bystander),
        bystander_before,
        "earlier calls' value transfers must be rolled back with the batch"
    );
    assert_eq!(
        fx.remaining_limit(root, session_key, token),
        U256::from(1_000u64),
        "no token deduction for a reverted batch"
    );
    assert_eq!(
        fx.remaining_limit(root, session_key, sentinel),
        U256::from(1_000_000u64),
        "no native-value/gas sentinel deduction either (signer paid gas)"
    );
}

// ====================================================================================
// transferFrom metering (M-2)
// ====================================================================================

/// Review finding M-2: the batch runs with `msg.sender == root`, so root IS the spender
/// of a top-level `transferFrom` — `transferFrom(from == root, …)` must count against
/// the per-token quota exactly like `transfer`.
#[test]
fn transferfrom_from_root_counts_against_quota() {
    let mut fx = AgentFixture::new();
    let (root, session_key, token, bystander) = (fx.root, fx.session_key, fx.token, fx.bystander);
    fx.authorize_key(root, session_key, vec![limit(token, 100)], true, vec![]);

    // Over-quota transferFrom(root → bystander) must be rejected in the pre-flight.
    let err = fx
        .send_aa_tx(
            Some(root),
            None,
            vec![call(token, 0, transfer_from_data(root, bystander, 150))],
        )
        .expect_err("transferFrom moving root's tokens above quota must be rejected");
    assert!(err.contains("rejected"), "unexpected error: {err}");

    // In-quota transferFrom(root → bystander) passes and deducts.
    let result = fx
        .send_aa_tx(Some(root), None, vec![call(token, 0, transfer_from_data(root, bystander, 60))])
        .expect("in-quota transferFrom must pass");
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        fx.remaining_limit(root, session_key, token),
        U256::from(40u64),
        "transferFrom(from == root) must deduct quota"
    );
}

/// A third-party `transferFrom` (`from != root`) spends someone else's allowance, not
/// root's balance — it is not metered against root's quota.
#[test]
fn transferfrom_third_party_not_metered() {
    let mut fx = AgentFixture::new();
    let (root, session_key, token, bystander) = (fx.root, fx.session_key, fx.token, fx.bystander);
    fx.authorize_key(root, session_key, vec![limit(token, 100)], true, vec![]);

    // Amount above root's quota, but from a third party — passes, no deduction.
    let result = fx
        .send_aa_tx(
            Some(root),
            None,
            vec![call(token, 0, transfer_from_data(bystander, root, 150))],
        )
        .expect("third-party transferFrom is not quota-gated");
    assert!(result.is_success(), "{result:?}");
    assert_eq!(
        fx.remaining_limit(root, session_key, token),
        U256::from(100u64),
        "third-party transferFrom must not deduct root's quota"
    );
}
