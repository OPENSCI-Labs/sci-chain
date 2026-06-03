use std::{
    any::Any,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use alloy_consensus::{BlockHeader, Transaction, Typed2718};
use alloy_primitives::{Address, U256};
use base_common_chains::Upgrades;
use base_common_evm::{BaseSpecId, L1BlockInfo};
use base_common_genesis::DaFootprintGasScalarUpdate;
use parking_lot::RwLock;
use reth_chainspec::ChainSpecProvider;
use reth_evm::ConfigureEvm;
use reth_primitives_traits::{
    Block, BlockBody, BlockTy, GotExpected, SealedBlock,
    transaction::error::InvalidTransactionError,
};
use reth_storage_api::{AccountInfoReader, AccountReader, BlockReaderIdExt, StateProviderFactory};
use reth_transaction_pool::{
    EthPoolTransaction, EthTransactionValidator, TransactionOrigin, TransactionValidationOutcome,
    TransactionValidator,
    error::{InvalidPoolTransactionError, PoolTransactionError},
};

use crate::BasePooledTx;

/// Base-specific transaction pool validation errors.
#[derive(Debug, thiserror::Error)]
pub enum BaseTxPoolError {
    /// The transaction's DA footprint exceeds the block gas limit.
    #[error(
        "transaction DA footprint ({transaction_da_footprint}) exceeds block gas limit ({block_gas_limit})"
    )]
    DaFootprintExceedsBlockGasLimit {
        /// The computed DA footprint of the transaction (`estimated_da_size` * `da_footprint_gas_scalar`).
        transaction_da_footprint: u64,
        /// The current block gas limit.
        block_gas_limit: u64,
    },
}

impl PoolTransactionError for BaseTxPoolError {
    fn is_bad_transaction(&self) -> bool {
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Tracks additional infos for the current block.
#[derive(Debug, Default)]
pub struct BaseL1BlockInfo {
    /// The current L1 block info.
    l1_block_info: RwLock<L1BlockInfo>,
    /// Current block timestamp.
    timestamp: AtomicU64,
}

impl BaseL1BlockInfo {
    /// Returns the most recent timestamp
    pub fn timestamp(&self) -> u64 {
        self.timestamp.load(Ordering::Relaxed)
    }
}

/// Validator for Base transactions.
#[derive(Debug, Clone)]
pub struct BaseTransactionValidator<Client, Tx, Evm> {
    /// The type that performs the actual validation.
    inner: Arc<EthTransactionValidator<Client, Tx, Evm>>,
    /// Additional block info required for validation.
    block_info: Arc<BaseL1BlockInfo>,
    /// If true, ensure that the transaction's sender has enough balance to cover the L1 gas fee
    /// derived from the tracked L1 block info that is extracted from the first transaction in the
    /// L2 block.
    require_l1_data_gas_fee: bool,
}

impl<Client, Tx, Evm> BaseTransactionValidator<Client, Tx, Evm> {
    /// Returns the configured chain spec
    pub fn chain_spec(&self) -> Arc<Client::ChainSpec>
    where
        Client: ChainSpecProvider,
    {
        self.inner.chain_spec()
    }

    /// Returns the configured client
    pub fn client(&self) -> &Client {
        self.inner.client()
    }

    /// Returns the current block timestamp.
    fn block_timestamp(&self) -> u64 {
        self.block_info.timestamp.load(Ordering::Relaxed)
    }

    /// Whether to ensure that the transaction's sender has enough balance to also cover the L1 gas
    /// fee.
    pub fn require_l1_data_gas_fee(self, require_l1_data_gas_fee: bool) -> Self {
        Self { require_l1_data_gas_fee, ..self }
    }

    /// Returns whether this validator also requires the transaction's sender to have enough balance
    /// to cover the L1 gas fee.
    pub const fn requires_l1_data_gas_fee(&self) -> bool {
        self.require_l1_data_gas_fee
    }
}

impl<Client, Tx, Evm> BaseTransactionValidator<Client, Tx, Evm>
where
    Client: ChainSpecProvider<ChainSpec: Upgrades> + StateProviderFactory + BlockReaderIdExt + Sync,
    Tx: EthPoolTransaction + BasePooledTx,
    Evm: ConfigureEvm,
{
    /// Create a new [`BaseTransactionValidator`].
    pub fn new(inner: EthTransactionValidator<Client, Tx, Evm>) -> Self {
        let this = Self::with_block_info(inner, BaseL1BlockInfo::default());
        if let Ok(Some(block)) =
            this.inner.client().block_by_number_or_tag(alloy_eips::BlockNumberOrTag::Latest)
        {
            // genesis block has no txs, so we can't extract L1 info, we set the block info to empty
            // so that we will accept txs into the pool before the first block
            if block.header().number() == 0 {
                this.block_info.timestamp.store(block.header().timestamp(), Ordering::Relaxed);
            } else {
                this.update_l1_block_info(block.header(), block.body().transactions().first());
            }
        }

        this
    }

    /// Create a new [`BaseTransactionValidator`] with the given [`BaseL1BlockInfo`].
    pub fn with_block_info(
        inner: EthTransactionValidator<Client, Tx, Evm>,
        block_info: BaseL1BlockInfo,
    ) -> Self {
        Self {
            inner: Arc::new(inner),
            block_info: Arc::new(block_info),
            require_l1_data_gas_fee: true,
        }
    }

    /// Update the L1 block info for the given header and system transaction, if any.
    ///
    /// Note: this supports optional system transaction, in case this is used in a dev setup
    pub fn update_l1_block_info<H, T>(&self, header: &H, tx: Option<&T>)
    where
        H: BlockHeader,
        T: Transaction,
    {
        self.block_info.timestamp.store(header.timestamp(), Ordering::Relaxed);

        if let Some(Ok(l1_block_info)) = tx.map(base_execution_evm::extract_l1_info_from_tx) {
            *self.block_info.l1_block_info.write() = l1_block_info;
        }
    }

    /// Validates a single transaction.
    ///
    /// See also [`TransactionValidator::validate_transaction`]
    ///
    /// This behaves the same as [`BaseTransactionValidator::validate_one_with_state`], but creates
    /// a new state provider internally.
    pub async fn validate_one(
        &self,
        origin: TransactionOrigin,
        transaction: Tx,
    ) -> TransactionValidationOutcome<Tx> {
        self.validate_one_with_state(origin, transaction, &mut None).await
    }

    /// Validates a single transaction with a provided state provider.
    ///
    /// This allows reusing the same state provider across multiple transaction validations.
    ///
    /// See also [`TransactionValidator::validate_transaction`]
    ///
    /// This behaves the same as [`EthTransactionValidator::validate_one_with_state`], but in
    /// addition applies Base-specific validity checks:
    /// - ensures tx is not eip4844
    /// - ensures that the account has enough balance to cover the L1 gas cost
    pub async fn validate_one_with_state(
        &self,
        origin: TransactionOrigin,
        transaction: Tx,
        state: &mut Option<Box<dyn AccountInfoReader + Send>>,
    ) -> TransactionValidationOutcome<Tx> {
        if transaction.is_eip4844() {
            return TransactionValidationOutcome::Invalid(
                transaction,
                InvalidTransactionError::TxTypeNotSupported.into(),
            );
        }

        // Plan A Phase 1: SCI AA transactions (type 0x76) are local-only in the sense that
        // they are never gossiped to peers (`propagate = false` below). We do NOT reject by
        // transaction origin: a sequencer running `--txpool.nolocals` tags every
        // RPC-submitted tx as `External`, so an origin-based reject would block legitimate
        // RPC ingress on exactly the node that must build AA blocks. Non-propagation is the
        // real "local-only" guarantee; admitting an AA tx from any origin is safe because it
        // still passes the keychain pre-execution hook at execution time.
        let is_aa = transaction.ty() == base_common_consensus::SCI_AA_TX_TYPE_ID;

        let outcome = self.inner.validate_one_with_state(origin, transaction, state);

        let mut outcome = self.apply_base_checks(outcome);

        // Plan A: for a sponsored AA tx the signer's pooled `cost` was overridden to its (zero)
        // obligation, so verify the `fee_payer` (== root) can actually cover the full economic
        // cost here — otherwise an underfunded sponsor would be admitted and only fail at block
        // build. No-op for non-AA / non-sponsored txs.
        outcome = self.check_aa_sponsor_balance(outcome, state);

        // Never propagate AA txs to peers (no alloy pooled-tx representation; SCI gossip of
        // AA is intentionally disabled).
        if is_aa {
            if let TransactionValidationOutcome::Valid { propagate, .. } = &mut outcome {
                *propagate = false;
            }
        }

        outcome
    }

    /// Performs the necessary Base-specific checks based on top of the regular eth outcome.
    fn apply_base_checks(
        &self,
        outcome: TransactionValidationOutcome<Tx>,
    ) -> TransactionValidationOutcome<Tx> {
        if !self.requires_l1_data_gas_fee() {
            // no need to check L1 gas fee
            return outcome;
        }
        // ensure that the account has enough balance to cover the L1 gas cost
        if let TransactionValidationOutcome::Valid {
            balance,
            state_nonce,
            transaction: valid_tx,
            propagate,
            bytecode_hash,
            authorities,
        } = outcome
        {
            let mut l1_block_info = self.block_info.l1_block_info.read().clone();

            // Check to ensure tx doesn't exceed the DA footprint limit
            if self.chain_spec().is_jovian_active_at_timestamp(self.block_timestamp()) {
                let da_footprint = valid_tx.transaction().estimated_da_size().saturating_mul(
                    l1_block_info
                        .da_footprint_gas_scalar
                        .unwrap_or(DaFootprintGasScalarUpdate::DEFAULT_DA_FOOTPRINT_GAS_SCALAR)
                        as u64,
                );
                let block_gas_limit = self.inner.block_gas_limit();
                if da_footprint > block_gas_limit {
                    return TransactionValidationOutcome::Invalid(
                        valid_tx.into_transaction(),
                        InvalidPoolTransactionError::other(
                            BaseTxPoolError::DaFootprintExceedsBlockGasLimit {
                                transaction_da_footprint: da_footprint,
                                block_gas_limit,
                            },
                        ),
                    );
                }
            }

            let encoded = valid_tx.transaction().encoded_2718();

            // Must mirror the execution-side cost in `BaseHandler` (L1 data fee + operator fee
            // post-Isthmus); otherwise operator-fee-underfunded txs get admitted but never execute.
            let spec_id = BaseSpecId::from_timestamp(self.chain_spec(), self.block_timestamp());
            let cost_addition = l1_block_info.tx_cost(
                &encoded,
                U256::from(valid_tx.transaction().gas_limit()),
                spec_id,
            );
            let cost = valid_tx.transaction().cost().saturating_add(cost_addition);

            // Plan A: a sponsored AA tx (`fee_payer != signer`) charges the signer only its
            // overridden pooled `cost` (0 when fully sponsored — see `aa_signer_cost`); the
            // sponsor's balance is verified separately in `check_aa_sponsor_balance`. Skip the
            // signer-balance check here so a fundless session key isn't rejected at admission.
            let aa_sponsored = valid_tx.transaction().aa().is_some_and(|aa| {
                aa.fee_payer.is_some_and(|fp| fp != valid_tx.transaction().sender())
            });

            // Checks for max cost (signer side)
            if !aa_sponsored && cost > balance {
                return TransactionValidationOutcome::Invalid(
                    valid_tx.into_transaction(),
                    InvalidTransactionError::InsufficientFunds(
                        GotExpected { got: balance, expected: cost }.into(),
                    )
                    .into(),
                );
            }

            return TransactionValidationOutcome::Valid {
                balance,
                state_nonce,
                transaction: valid_tx,
                propagate,
                bytecode_hash,
                authorities,
            };
        }
        outcome
    }

    /// For a sponsored AA tx (`fee_payer != signer`; `fee_payer == root` per the handler),
    /// verifies the fee_payer can cover the full economic cost (L2 gas + Σ call values + L1
    /// data fee). The signer's own balance is irrelevant — its pooled `cost` was overridden to
    /// its (zero) obligation by `AaTx::aa_signer_pool_cost` (see `transaction.rs`). No-op for
    /// non-AA or non-sponsored transactions.
    ///
    /// Note: the pool does not track the fee_payer's balance across blocks, so a fee_payer that
    /// drains *after* admission is not evicted — the tx stays pending and is skipped at block
    /// build (the handler rejects it). Same class as any external dependency.
    fn check_aa_sponsor_balance(
        &self,
        outcome: TransactionValidationOutcome<Tx>,
        state: &Option<Box<dyn AccountInfoReader + Send>>,
    ) -> TransactionValidationOutcome<Tx> {
        let TransactionValidationOutcome::Valid {
            balance,
            state_nonce,
            transaction: valid_tx,
            propagate,
            bytecode_hash,
            authorities,
        } = outcome
        else {
            return outcome;
        };

        // Extract (fee_payer, gas_cost, Σvalue, gas_limit) for a sponsored AA tx; pass through otherwise.
        let sender = valid_tx.transaction().sender();
        let sponsor = valid_tx.transaction().aa().and_then(|aa| {
            aa.fee_payer.filter(|fp| *fp != sender).map(|fp| {
                let gas = U256::from(aa.max_fee_per_gas).saturating_mul(U256::from(aa.gas_limit));
                let value =
                    aa.calls.iter().fold(U256::ZERO, |acc, c| acc.saturating_add(c.value));
                (fp, gas, value, aa.gas_limit)
            })
        });
        let Some((fee_payer, gas_cost, value, gas_limit)) = sponsor else {
            return TransactionValidationOutcome::Valid {
                balance,
                state_nonce,
                transaction: valid_tx,
                propagate,
                bytecode_hash,
                authorities,
            };
        };

        // Full economic cost the fee_payer bears: L2 gas + Σvalue + L1 data fee.
        let l1_cost = if self.requires_l1_data_gas_fee() {
            let mut l1_block_info = self.block_info.l1_block_info.read().clone();
            let spec_id = BaseSpecId::from_timestamp(self.chain_spec(), self.block_timestamp());
            l1_block_info.tx_cost(
                &valid_tx.transaction().encoded_2718(),
                U256::from(gas_limit),
                spec_id,
            )
        } else {
            U256::ZERO
        };
        let needed = gas_cost.saturating_add(value).saturating_add(l1_cost);
        let fee_payer_balance = self.account_balance(fee_payer, state);

        if fee_payer_balance < needed {
            return TransactionValidationOutcome::Invalid(
                valid_tx.into_transaction(),
                InvalidTransactionError::InsufficientFunds(
                    GotExpected { got: fee_payer_balance, expected: needed }.into(),
                )
                .into(),
            );
        }

        TransactionValidationOutcome::Valid {
            balance,
            state_nonce,
            transaction: valid_tx,
            propagate,
            bytecode_hash,
            authorities,
        }
    }

    /// Reads an account's balance from the in-flight validation `state` reader, falling back to
    /// the latest committed state. Returns zero when the account is absent or unreadable.
    fn account_balance(
        &self,
        address: Address,
        state: &Option<Box<dyn AccountInfoReader + Send>>,
    ) -> U256 {
        if let Some(reader) = state.as_ref() {
            if let Ok(Some(account)) = reader.basic_account(&address) {
                return account.balance;
            }
        }
        self.client()
            .latest()
            .ok()
            .and_then(|sp| sp.basic_account(&address).ok().flatten())
            .map(|account| account.balance)
            .unwrap_or(U256::ZERO)
    }
}

impl<Client, Tx, Evm> TransactionValidator for BaseTransactionValidator<Client, Tx, Evm>
where
    Client: ChainSpecProvider<ChainSpec: Upgrades> + StateProviderFactory + BlockReaderIdExt + Sync,
    Tx: EthPoolTransaction + BasePooledTx,
    Evm: ConfigureEvm,
{
    type Transaction = Tx;
    type Block = BlockTy<Evm::Primitives>;

    async fn validate_transaction(
        &self,
        origin: TransactionOrigin,
        transaction: Self::Transaction,
    ) -> TransactionValidationOutcome<Self::Transaction> {
        self.validate_one(origin, transaction).await
    }

    fn on_new_head_block(&self, new_tip_block: &SealedBlock<Self::Block>) {
        self.inner.on_new_head_block(new_tip_block);
        self.update_l1_block_info(
            new_tip_block.header(),
            new_tip_block.body().transactions().first(),
        );
    }
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{SignableTransaction, TxEip1559, transaction::SignerRecoverable};
    use alloy_eips::eip2718::Encodable2718;
    use alloy_primitives::{Address, TxKind, U256, bytes, hex::decode};
    use alloy_signer::SignerSync;
    use alloy_consensus::transaction::Recovered;
    use base_common_chains::ChainConfig;
    use base_common_consensus::{
        BaseAaTransaction, BasePrimitives, BaseTransactionSigned, BaseTxEnvelope, Call, TxDeposit,
    };
    use base_execution_chainspec::BaseChainSpec;
    use base_execution_evm::BaseEvmConfig;
    use base_test_utils::Account;
    use reth_provider::test_utils::{ExtendedAccount, MockEthProvider};
    use reth_transaction_pool::{
        PoolTransaction, TransactionOrigin, TransactionValidationOutcome,
        blobstore::InMemoryBlobStore, validate::EthTransactionValidatorBuilder,
    };

    use super::*;
    use crate::BasePooledTransaction;

    /// L1 attribute deposit calldata that activates Isthmus and seeds a non-zero
    /// `operator_fee_scalar`/`operator_fee_constant`. Mirrors the fixture used by
    /// `parse_l1_info_isthmus` in `crates/execution/evm/src/l1.rs`.
    const ISTHMUS_L1_INFO_DATA_HEX: &str = concat!(
        "098999be00000558000c5fc500000000000000030000000067a9f765",
        "0000000000000029000000000000000000000000000000000000000000000000",
        "00000000006a6d090000000000000000000000000000000000000000000000000000000000000001",
        "72fcc8e8886636bdbe96ba0e4baab67ea7e7811633f52b52e8cf7a5123213b6f",
        "000000000000000000000000d3f2c5afb2d76f5579f326b0cd7da5f5a4126c35",
        "00004e2000000000000001f4",
    );

    /// Regression test for `HackerOne` #74725.
    ///
    /// Asserts that the txpool affordability check accounts for the post-Isthmus operator fee, so a
    /// sender funded only for `tx.cost + l1_data_fee` (but not the additional operator fee) is
    /// rejected at admission instead of being accepted and later failing during execution with
    /// `LackOfFundForMaxFee`.
    #[tokio::test]
    async fn rejects_tx_underfunded_for_operator_fee_post_isthmus() {
        let chain_config = ChainConfig::mainnet();
        let chain_spec = Arc::new(BaseChainSpec::mainnet());

        let signer = Account::Alice.signer();
        let sender = signer.address();
        let tx = TxEip1559 {
            chain_id: chain_config.chain_id,
            nonce: 0,
            gas_limit: 50_000,
            max_fee_per_gas: 1_000,
            max_priority_fee_per_gas: 0,
            to: TxKind::Call(Address::random()),
            value: U256::ZERO,
            access_list: Default::default(),
            input: bytes!("FACADE"),
        };
        let gas_limit = tx.gas_limit;
        let signature = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
        let envelope = BaseTxEnvelope::Eip1559(tx.into_signed(signature));
        let recovered_tx = envelope.clone().try_into_recovered().unwrap();
        let encoded = recovered_tx.encoded_2718();

        let isthmus_data = decode(ISTHMUS_L1_INFO_DATA_HEX).expect("valid hex fixture");
        let mut l1_block_info = base_execution_evm::parse_l1_info(&isthmus_data).unwrap();
        let l1_only_cost = base_execution_evm::RethL1BlockInfo::l1_tx_data_fee(
            &mut l1_block_info,
            Arc::clone(&chain_spec),
            chain_config.isthmus_timestamp,
            &encoded,
            false,
        )
        .unwrap();
        let full_additional_cost = l1_block_info.tx_cost(
            &encoded,
            U256::from(gas_limit),
            BaseSpecId::from_timestamp(Arc::clone(&chain_spec), chain_config.isthmus_timestamp),
        );
        let base_tx_cost = U256::from(envelope.value()).saturating_add(U256::from(
            envelope.max_fee_per_gas().saturating_mul(envelope.gas_limit() as u128),
        ));
        let balance = base_tx_cost.saturating_add(l1_only_cost);

        assert!(
            full_additional_cost > l1_only_cost,
            "fixture must produce a non-zero operator fee post-Isthmus"
        );
        assert!(
            base_tx_cost.saturating_add(full_additional_cost) > balance,
            "balance must be insufficient once the operator fee is included"
        );

        let client = MockEthProvider::<BasePrimitives>::new()
            .with_chain_spec(Arc::clone(&chain_spec))
            .with_genesis_block();
        client.add_account(sender, ExtendedAccount::new(0, balance));
        let evm_config = BaseEvmConfig::base(Arc::clone(&chain_spec));
        let inner = EthTransactionValidatorBuilder::new(client, evm_config)
            .no_shanghai()
            .no_cancun()
            .with_custom_tx_type(base_common_consensus::SCI_AA_TX_TYPE_ID)
            .build(InMemoryBlobStore::default());
        let validator =
            BaseTransactionValidator::with_block_info(inner, BaseL1BlockInfo::default());

        let header = alloy_consensus::Header {
            timestamp: chain_config.isthmus_timestamp,
            ..Default::default()
        };
        let l1_info_tx: BaseTransactionSigned = TxDeposit {
            source_hash: Default::default(),
            from: Address::ZERO,
            to: TxKind::Create,
            mint: 0,
            value: U256::ZERO,
            gas_limit: 0,
            is_system_transaction: false,
            input: isthmus_data.into(),
        }
        .into();
        validator.update_l1_block_info(&header, Some(&l1_info_tx));

        let pooled_tx: BasePooledTransaction =
            BasePooledTransaction::new(recovered_tx, envelope.encode_2718_len());
        let outcome = validator.validate_one(TransactionOrigin::External, pooled_tx).await;

        match outcome {
            TransactionValidationOutcome::Invalid(_, err) => {
                assert!(
                    matches!(
                        err,
                        InvalidPoolTransactionError::Consensus(
                            InvalidTransactionError::InsufficientFunds(_)
                        )
                    ),
                    "expected InsufficientFunds, got: {err:?}"
                );
            }
            other => panic!(
                "expected operator-fee-underfunded tx to be rejected at admission, got {other:?}"
            ),
        }
    }

    /// gas_cost of the AA fixture (`gas_limit * max_fee_per_gas`).
    const AA_GAS_COST: u128 = 50_000 * 1_000;

    /// Builds a signed SCI AA (`0x76`) transaction (signed by Alice = the session key) and its
    /// recovered form, with the given `fee_payer` / `root` and a single call moving `value`.
    fn build_aa_tx(
        chain_id: u64,
        fee_payer: Option<Address>,
        root: Option<Address>,
        value: U256,
    ) -> (Recovered<BaseTransactionSigned>, usize) {
        let signer = Account::Alice.signer();
        let tx = BaseAaTransaction {
            chain_id,
            nonce: 0,
            max_priority_fee_per_gas: 0,
            max_fee_per_gas: 1_000,
            gas_limit: 50_000,
            calls: vec![Call {
                to: TxKind::Call(Address::random()),
                value,
                input: Default::default(),
            }],
            access_list: Default::default(),
            fee_payer,
            root,
        };
        let signature = signer.sign_hash_sync(&tx.signature_hash()).unwrap();
        let envelope = BaseTxEnvelope::Aa(tx.into_signed(signature));
        let len = envelope.encode_2718_len();
        (envelope.try_into_recovered().unwrap(), len)
    }

    /// A fully-sponsored AA tx (`fee_payer == root != signer`) is admitted even though the
    /// signing session key holds no funds — the fee_payer covers the cost.
    #[tokio::test]
    async fn admits_fundless_signer_when_fee_payer_funded() {
        let chain_spec = Arc::new(BaseChainSpec::mainnet());
        let chain_id = ChainConfig::mainnet().chain_id;
        let sponsor = Address::random(); // fee_payer == root
        let (recovered, len) = build_aa_tx(chain_id, Some(sponsor), Some(sponsor), U256::from(5));
        let signer = recovered.signer();

        let client = MockEthProvider::<BasePrimitives>::new()
            .with_chain_spec(Arc::clone(&chain_spec))
            .with_genesis_block();
        client.add_account(signer, ExtendedAccount::new(0, U256::ZERO)); // fundless session key
        client.add_account(
            sponsor,
            ExtendedAccount::new(0, U256::from(1_000_000_000_000_000_000u128)), // 1 ETH
        );
        let evm_config = BaseEvmConfig::base(Arc::clone(&chain_spec));
        let inner = EthTransactionValidatorBuilder::new(client, evm_config)
            .no_shanghai()
            .no_cancun()
            .with_custom_tx_type(base_common_consensus::SCI_AA_TX_TYPE_ID)
            .build(InMemoryBlobStore::default());
        let validator =
            BaseTransactionValidator::with_block_info(inner, BaseL1BlockInfo::default());

        let pooled: BasePooledTransaction = BasePooledTransaction::new(recovered, len);
        assert_eq!(*pooled.cost(), U256::ZERO, "fully-sponsored signer owes nothing");
        let outcome = validator.validate_one(TransactionOrigin::External, pooled).await;
        assert!(
            matches!(outcome, TransactionValidationOutcome::Valid { .. }),
            "fundless signer with funded fee_payer should be admitted, got {outcome:?}"
        );
    }

    /// A sponsored AA tx whose `fee_payer` cannot cover the cost is rejected at admission
    /// (rather than admitted and silently failing at block build).
    #[tokio::test]
    async fn rejects_aa_when_fee_payer_underfunded() {
        let chain_spec = Arc::new(BaseChainSpec::mainnet());
        let chain_id = ChainConfig::mainnet().chain_id;
        let sponsor = Address::random();
        let (recovered, len) = build_aa_tx(chain_id, Some(sponsor), Some(sponsor), U256::from(5));
        let signer = recovered.signer();

        let client = MockEthProvider::<BasePrimitives>::new()
            .with_chain_spec(Arc::clone(&chain_spec))
            .with_genesis_block();
        client.add_account(signer, ExtendedAccount::new(0, U256::ZERO));
        client.add_account(sponsor, ExtendedAccount::new(0, U256::from(1_000))); // << gas cost
        let evm_config = BaseEvmConfig::base(Arc::clone(&chain_spec));
        let inner = EthTransactionValidatorBuilder::new(client, evm_config)
            .no_shanghai()
            .no_cancun()
            .with_custom_tx_type(base_common_consensus::SCI_AA_TX_TYPE_ID)
            .build(InMemoryBlobStore::default());
        let validator =
            BaseTransactionValidator::with_block_info(inner, BaseL1BlockInfo::default());

        let pooled: BasePooledTransaction = BasePooledTransaction::new(recovered, len);
        let outcome = validator.validate_one(TransactionOrigin::External, pooled).await;
        match outcome {
            TransactionValidationOutcome::Invalid(_, err) => assert!(
                matches!(
                    err,
                    InvalidPoolTransactionError::Consensus(
                        InvalidTransactionError::InsufficientFunds(_)
                    )
                ),
                "expected InsufficientFunds for underfunded fee_payer, got {err:?}"
            ),
            other => panic!("expected underfunded fee_payer to be rejected, got {other:?}"),
        }
    }

    /// The pooled `cost` reflects what the *signer* actually owes: zero when fully sponsored,
    /// gas + Σvalue for a plain (`root=None`/`fee_payer=None`) batch.
    #[test]
    fn aa_pool_cost_reflects_signer_obligation() {
        let chain_id = ChainConfig::mainnet().chain_id;
        let sponsor = Address::random();

        let (sponsored, len) = build_aa_tx(chain_id, Some(sponsor), Some(sponsor), U256::from(5));
        let pooled: BasePooledTransaction = BasePooledTransaction::new(sponsored, len);
        assert_eq!(*pooled.cost(), U256::ZERO, "fully sponsored → signer cost 0");

        let (plain, len2) = build_aa_tx(chain_id, None, None, U256::from(5));
        let pooled2: BasePooledTransaction = BasePooledTransaction::new(plain, len2);
        assert_eq!(
            *pooled2.cost(),
            U256::from(AA_GAS_COST).saturating_add(U256::from(5)),
            "plain batch → signer owes gas + value"
        );
    }
}
