use alloy_evm::EvmInternals;
use alloy_primitives::{Address, Log, LogData, U256};
use revm::{
    // SCI patch — bring the v38 state-gas extension trait into scope so
    // `gas_params.code_deposit_state_gas(...)` / `create_state_gas()` /
    // `sstore_state_gas(...)` calls below resolve to the shim's no-op stubs.
    // Re-apply on every Tempo sync.
    GasParamsExt,
    context::{Block, CfgEnv, journaled_state::JournalCheckpoint},
    context_interface::cfg::{GasParams, gas},
    interpreter::gas::GasTracker,
    state::{AccountInfo, Bytecode},
};
use tempo_chainspec::hardfork::TempoHardfork;

use crate::{error::TempoPrecompileError, storage::PrecompileStorageProvider};

/// Production [`PrecompileStorageProvider`] backed by the live EVM journal.
///
/// Wraps `EvmInternals` and tracks gas consumption for storage operations.
pub struct EvmPrecompileStorageProvider<'a> {
    internals: EvmInternals<'a>,
    gas_tracker: GasTracker,
    spec: TempoHardfork,
    amsterdam_eip8037_enabled: bool,
    is_static: bool,
    gas_params: GasParams,
    /// Debug-only LIFO checkpoint validator. See [`Self::assert_lifo`].
    #[cfg(debug_assertions)]
    checkpoint_stack: Vec<(usize, usize)>,
}

impl<'a> EvmPrecompileStorageProvider<'a> {
    /// Creates a new storage provider with the given gas limit, hardfork, and static flag.
    pub const fn new(
        internals: EvmInternals<'a>,
        gas_limit: u64,
        reservoir: u64,
        spec: TempoHardfork,
        amsterdam_eip8037_enabled: bool,
        is_static: bool,
        gas_params: GasParams,
    ) -> Self {
        Self {
            internals,
            gas_tracker: GasTracker::new(gas_limit, gas_limit, reservoir),
            spec,
            amsterdam_eip8037_enabled,
            is_static,
            gas_params,
            #[cfg(debug_assertions)]
            checkpoint_stack: Vec::new(),
        }
    }

    /// Creates a new storage provider with maximum gas limit and non-static context.
    pub fn new_max_gas(internals: EvmInternals<'a>, cfg: &CfgEnv<TempoHardfork>) -> Self {
        // SCI patch — revm 34's `CfgEnv` has no `enable_amsterdam_eip8037`
        // field. Hardcoded `false` because SCI does not adopt EIP-8037
        // (TIP-1016) state-gas accounting. Re-apply on every Tempo sync.
        Self::new(internals, u64::MAX, 0, cfg.spec, false, false, cfg.gas_params.clone())
    }

    /// Creates a new storage provider with the given gas limit, deriving spec from `cfg`.
    pub fn new_with_gas_limit(
        internals: EvmInternals<'a>,
        cfg: &CfgEnv<TempoHardfork>,
        gas_limit: u64,
        reservoir: u64,
    ) -> Self {
        // SCI patch — see `new_max_gas` above.
        Self::new(internals, gas_limit, reservoir, cfg.spec, false, false, cfg.gas_params.clone())
    }

    #[inline]
    const fn deduct_state_gas(&mut self, gas: u64) -> Result<(), TempoPrecompileError> {
        if !self.gas_tracker.record_state_cost(gas) {
            return Err(TempoPrecompileError::OutOfGas);
        }
        Ok(())
    }
}

impl<'a> PrecompileStorageProvider for EvmPrecompileStorageProvider<'a> {
    fn chain_id(&self) -> u64 {
        self.internals.chain_id()
    }

    fn timestamp(&self) -> U256 {
        self.internals.block_timestamp()
    }

    fn beneficiary(&self) -> Address {
        self.internals.block_env().beneficiary()
    }

    fn block_number(&self) -> u64 {
        self.internals.block_env().number().to::<u64>()
    }

    #[inline]
    fn set_code(&mut self, address: Address, code: Bytecode) -> Result<(), TempoPrecompileError> {
        let code_len = code.len();
        self.deduct_gas(self.gas_params.code_deposit_cost(code_len))?;

        // Track state gas for code deposit
        self.deduct_state_gas(self.gas_params.code_deposit_state_gas(code_len))?;

        let was_empty = {
            let mut account = self.internals.load_account_mut(address)?;
            let was_empty = account.data.account().info.is_empty();
            account.set_code_and_hash_slow(code);
            was_empty
        };

        // TIP-1016: charge TIP20 deployments as CREATE.
        if self.amsterdam_eip8037_enabled && was_empty {
            self.deduct_gas(self.gas_params.create_cost())?;
            self.deduct_state_gas(self.gas_params.create_state_gas())?;
            self.deduct_gas(self.gas_params.keccak256_cost(code_len.div_ceil(32)))?;
        }

        Ok(())
    }

    #[inline]
    fn with_account_info(
        &mut self,
        address: Address,
        f: &mut dyn FnMut(&AccountInfo),
    ) -> Result<(), TempoPrecompileError> {
        let additional_cost = self.gas_params.cold_account_additional_cost();

        // T4+: pre-charge static gas to avoid cheap useless work.
        let insufficient_gas_for_cold_load = if self.spec.is_t4() {
            self.deduct_gas(self.gas_params.warm_storage_read_cost())?;
            self.gas_tracker.remaining() < additional_cost
        } else {
            false
        };

        let mut account = self
            .internals
            .load_account_mut_skip_cold_load(address, insufficient_gas_for_cold_load)?;

        if !self.spec.is_t4() {
            deduct_gas(&mut self.gas_tracker, self.gas_params.warm_storage_read_cost())?;
        }

        // dynamic gas
        if account.is_cold {
            deduct_gas(&mut self.gas_tracker, additional_cost)?;
        }

        account.load_code()?;

        f(&account.data.account().info);
        Ok(())
    }

    #[inline]
    fn sstore(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), TempoPrecompileError> {
        // T4+: pre-charge static gas before loading storage to avoid cheap useless work.
        let insufficient_gas_for_cold_load = if self.spec.is_t4() {
            self.deduct_gas(self.gas_params.sstore_static_gas())?;
            self.gas_tracker.remaining() < self.gas_params.cold_storage_additional_cost()
        } else {
            false
        };

        let result = self.internals.load_account_mut(address)?.sstore(
            key,
            value,
            insufficient_gas_for_cold_load,
        )?;

        if !self.spec.is_t4() {
            self.deduct_gas(self.gas_params.sstore_static_gas())?;
        }

        // dynamic gas
        self.deduct_gas(self.gas_params.sstore_dynamic_gas(true, &result.data, result.is_cold))?;

        // Track state gas (cold SSTORE zero->non-zero only)
        self.deduct_state_gas(self.gas_params.sstore_state_gas(&result.data))?;

        // refund gas.
        self.refund_gas(self.gas_params.sstore_refund(true, &result.data));

        Ok(())
    }

    #[inline]
    fn tstore(
        &mut self,
        address: Address,
        key: U256,
        value: U256,
    ) -> Result<(), TempoPrecompileError> {
        self.deduct_gas(self.gas_params.warm_storage_read_cost())?;
        self.internals.tstore(address, key, value);
        Ok(())
    }

    #[inline]
    fn emit_event(&mut self, address: Address, event: LogData) -> Result<(), TempoPrecompileError> {
        self.deduct_gas(
            gas::LOG
                + self.gas_params.log_cost(event.topics().len() as u8, event.data.len() as u64),
        )?;

        self.internals.log(Log { address, data: event });

        Ok(())
    }

    #[inline]
    fn sload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        let additional_cost = self.gas_params.cold_storage_additional_cost();

        // T4+: pre-charge static gas to avoid cheap useless work.
        let insufficient_gas_for_cold_load = if self.spec.is_t4() {
            self.deduct_gas(self.gas_params.warm_storage_read_cost())?;
            self.gas_tracker.remaining() < additional_cost
        } else {
            false
        };

        let value;
        let is_cold;
        {
            let mut account = self.internals.load_account_mut(address)?;
            let val = account.sload(key, insufficient_gas_for_cold_load)?;

            value = val.present_value;
            is_cold = val.is_cold;
        };

        if !self.spec.is_t4() {
            self.deduct_gas(self.gas_params.warm_storage_read_cost())?;
        }

        // dynamic gas
        if is_cold {
            self.deduct_gas(additional_cost)?;
        }

        Ok(value)
    }

    #[inline]
    fn tload(&mut self, address: Address, key: U256) -> Result<U256, TempoPrecompileError> {
        self.deduct_gas(self.gas_params.warm_storage_read_cost())?;

        Ok(self.internals.tload(address, key))
    }

    #[inline]
    fn deduct_gas(&mut self, gas: u64) -> Result<(), TempoPrecompileError> {
        deduct_gas(&mut self.gas_tracker, gas)
    }

    #[inline]
    fn refund_gas(&mut self, gas: i64) {
        self.gas_tracker.record_refund(gas);
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        self.gas_tracker.limit()
    }

    #[inline]
    fn gas_used(&self) -> u64 {
        self.gas_tracker.limit() - self.gas_tracker.remaining()
    }

    #[inline]
    fn state_gas_used(&self) -> u64 {
        self.gas_tracker.state_gas_spent()
    }

    #[inline]
    fn gas_refunded(&self) -> i64 {
        self.gas_tracker.refunded()
    }

    #[inline]
    fn reservoir(&self) -> u64 {
        self.gas_tracker.reservoir()
    }

    #[inline]
    fn spec(&self) -> TempoHardfork {
        self.spec
    }

    #[inline]
    fn amsterdam_eip8037_enabled(&self) -> bool {
        self.amsterdam_eip8037_enabled
    }

    #[inline]
    fn is_static(&self) -> bool {
        self.is_static
    }

    #[inline]
    fn checkpoint(&mut self) -> JournalCheckpoint {
        let cp = self.internals.checkpoint();
        #[cfg(debug_assertions)]
        self.track_checkpoint(&cp);
        cp
    }

    #[inline]
    fn checkpoint_commit(&mut self, _checkpoint: JournalCheckpoint) {
        #[cfg(debug_assertions)]
        self.assert_lifo(&_checkpoint, "commit");
        self.internals.checkpoint_commit()
    }

    #[inline]
    fn checkpoint_revert(&mut self, checkpoint: JournalCheckpoint) {
        #[cfg(debug_assertions)]
        self.assert_lifo(&checkpoint, "revert");
        self.internals.checkpoint_revert(checkpoint)
    }
}

/// LIFO checkpoint validation (debug builds only).
///
/// Since `EvmInternals` doesn't expose revm's journal depth, we mirror it by
/// recording each checkpoint's (`journal_i`, `log_i`) on creation and asserting
/// that commits/reverts always resolve the most recent checkpoint first.
#[cfg(debug_assertions)]
impl EvmPrecompileStorageProvider<'_> {
    /// Records a newly created checkpoint for later LIFO validation.
    fn track_checkpoint(&mut self, cp: &JournalCheckpoint) {
        self.checkpoint_stack.push((cp.journal_i, cp.log_i));
    }

    /// Panics if `cp` is not the most recently created checkpoint.
    fn assert_lifo(&mut self, cp: &JournalCheckpoint, op: &str) {
        let top = self
            .checkpoint_stack
            .pop()
            .unwrap_or_else(|| panic!("checkpoint_{op}: no active checkpoint"));

        assert_eq!(
            (cp.journal_i, cp.log_i),
            top,
            "out-of-order checkpoint {op} (expected top of stack)"
        );
    }
}

/// Deducts gas from the remaining gas and returns an error if insufficient.
#[inline]
pub const fn deduct_gas(
    gas_tracker: &mut GasTracker,
    additional_cost: u64,
) -> Result<(), TempoPrecompileError> {
    if !gas_tracker.record_regular_cost(additional_cost) {
        return Err(TempoPrecompileError::OutOfGas);
    }
    Ok(())
}
