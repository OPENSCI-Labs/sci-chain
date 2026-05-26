//! Shim for revm 38's `interpreter::gas::GasTracker` (introduced by EIP-8037 /
//! TIP-1016 for state-gas reservoir accounting). Real revm 34 has no
//! equivalent. Verbatim Tempo v1.7.1 storage layer reads/writes the tracker
//! unconditionally even when `amsterdam_eip8037_enabled = false`, so the shim
//! provides a no-op stand-in that compiles but never advances state-gas
//! counters. All counters remain at 0 and remain at 0 — fine for SCI because
//! we don't expose reservoir to any downstream consumer.

// Re-export everything from real revm 34's interpreter module verbatim. The
// only addition is the [`gas::GasTracker`] stub.
pub use revm::interpreter::*;

/// Shim for revm 38's `revm::interpreter::gas` module — re-exports revm 34's
/// gas submodule verbatim and adds a no-op [`GasTracker`] stub.
pub mod gas {
    pub use revm::interpreter::gas::*;

    /// No-op stand-in for revm 38's `GasTracker`. SCI does not adopt EIP-8037,
    /// so state-gas counters are not consulted by any downstream code path. The
    /// stub exists only so verbatim Tempo source compiles.
    ///
    /// All accessors return `0`; `deduct_state_gas` is a no-op that always
    /// succeeds. If a future v1.7.x sync starts *enforcing* reservoir checks
    /// at runtime, this stub will need real semantics.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct GasTracker {
        gas_limit: u64,
        gas_used: u64,
        state_gas_used: u64,
        reservoir: u64,
        gas_refunded: i64,
    }

    impl GasTracker {
        /// Constructs a tracker mirroring revm 38's signature.
        pub const fn new(gas_limit: u64, _initial_gas: u64, reservoir: u64) -> Self {
            Self { gas_limit, gas_used: 0, state_gas_used: 0, reservoir, gas_refunded: 0 }
        }

        /// Returns the tracker's gas limit.
        pub const fn limit(&self) -> u64 {
            self.gas_limit
        }

        /// Alias for [`Self::limit`]; v1.7.1 keychain source uses both names.
        pub const fn gas_limit(&self) -> u64 {
            self.gas_limit
        }

        /// Returns total ordinary gas used.
        pub const fn gas_used(&self) -> u64 {
            self.gas_used
        }

        /// Returns remaining ordinary gas (limit minus used).
        pub const fn remaining(&self) -> u64 {
            self.gas_limit.saturating_sub(self.gas_used)
        }

        /// Returns the state-gas portion of total gas used. SCI: always 0.
        pub const fn state_gas_used(&self) -> u64 {
            self.state_gas_used
        }

        /// Alias for [`Self::state_gas_used`].
        pub const fn state_gas_spent(&self) -> u64 {
            self.state_gas_used
        }

        /// Returns the reservoir balance. SCI: passthrough of constructor arg, no updates.
        pub const fn reservoir(&self) -> u64 {
            self.reservoir
        }

        /// Returns the gas refunded so far.
        pub const fn gas_refunded(&self) -> i64 {
            self.gas_refunded
        }

        /// Alias for [`Self::gas_refunded`].
        pub const fn refunded(&self) -> i64 {
            self.gas_refunded
        }

        /// No-op for SCI. Real revm 38 would deduct against the reservoir.
        pub fn deduct_state_gas(&mut self, _amount: u64) -> Result<(), GasTrackerError> {
            Ok(())
        }

        /// Records state-creating cost. SCI: always returns `true` (no enforcement).
        pub fn record_state_cost(&mut self, _amount: u64) -> bool {
            true
        }

        /// Records ordinary gas cost. Returns `false` on insufficient balance.
        pub fn record_regular_cost(&mut self, amount: u64) -> bool {
            let new_used = self.gas_used.saturating_add(amount);
            if new_used > self.gas_limit {
                return false;
            }
            self.gas_used = new_used;
            true
        }

        /// Records a gas refund.
        pub fn record_refund(&mut self, amount: i64) {
            self.gas_refunded = self.gas_refunded.saturating_add(amount);
        }

        /// Deducts ordinary gas. Returns `Err` on insufficient balance.
        pub fn deduct_gas(&mut self, amount: u64) -> Result<(), GasTrackerError> {
            let new_used = self.gas_used.saturating_add(amount);
            if new_used > self.gas_limit {
                return Err(GasTrackerError::OutOfGas);
            }
            self.gas_used = new_used;
            Ok(())
        }

        /// Adds to the refund counter.
        pub fn refund_gas(&mut self, amount: i64) {
            self.gas_refunded = self.gas_refunded.saturating_add(amount);
        }
    }

    /// Error type for [`GasTracker`] operations.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum GasTrackerError {
        /// Insufficient gas remaining.
        OutOfGas,
    }
}
