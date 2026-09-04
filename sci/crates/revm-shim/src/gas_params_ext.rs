//! Extension trait that bolts revm 38's state-gas accounting methods onto
//! revm 34's `GasParams`. SCI does not adopt EIP-8037 / TIP-1016 — all three
//! methods return `0`, so any verbatim Tempo source that calls them (e.g.
//! `gas_params.code_deposit_state_gas(len)`) ends up tracking no state gas.
//!
//! ## Why an extension trait
//!
//! Real revm 34's `GasParams` is a plain struct with an inherent-method API.
//! v38 added three state-gas methods to that same inherent impl block. We
//! can't add inherent methods to a foreign struct from outside its defining
//! crate, but we can add a trait with default methods that callers bring into
//! scope. Verbatim Tempo source files that call `self.gas_params.create_state_gas()`
//! need a one-line `use sci_revm_shim::GasParamsExt;` (or, since `revm` is
//! aliased to the shim, `use revm::GasParamsExt;`) injected at SCI-sync time.
//! That single import is documented in CLAUDE.md as one of the recurring SCI
//! patches.

use revm::context_interface::cfg::GasParams;

/// Provides v38 state-gas accessors as no-op stubs returning `0` for any input.
pub trait GasParamsExt {
    /// EIP-8037 state-gas for a code deposit of the given size in bytes.
    /// SCI: always `0`.
    fn code_deposit_state_gas(&self, _code_len: usize) -> u64 {
        0
    }

    /// EIP-8037 state-gas for a CREATE / CREATE2. SCI: always `0`.
    fn create_state_gas(&self) -> u64 {
        0
    }

    /// EIP-8037 state-gas for an SSTORE operation. SCI: always `0`.
    /// The `_result` parameter accepts any value — the shim doesn't introspect
    /// it because no state-gas is charged anyway.
    fn sstore_state_gas<R>(&self, _result: R) -> u64 {
        0
    }
}

impl GasParamsExt for GasParams {}
