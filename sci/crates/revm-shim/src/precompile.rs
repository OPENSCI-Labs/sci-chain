//! Shim for revm 38's `PrecompileOutput` / `PrecompileHalt` API on revm 34.
//!
//! See the crate-level docs in `lib.rs` for the wiring rationale. This module
//! re-exports every item from `revm::precompile` *except* `PrecompileOutput`
//! and `PrecompileResult`, which are replaced with SCI shim newtypes that
//! carry the v38-shape fields (`state_gas_used`, `reservoir`, `status`) plus
//! `::halt(...)` / `::revert(...)` / `::new(...)` constructors that take a
//! trailing `reservoir` parameter.

use alloy_primitives::Bytes;

// Re-export the non-shadowed surface of revm 34's `precompile` module
// verbatim. Tempo verbatim source paths like `revm::precompile::PrecompileError`
// or `revm::precompile::PrecompileId` resolve through here.
pub use revm::precompile::{
    blake2, bls12_381, bls12_381_const, bls12_381_utils, bn254, calc_linear_cost, crypto, hash,
    identity, install_crypto, interface, kzg_point_evaluation, modexp, secp256k1, secp256r1,
    utilities, Crypto, DefaultCrypto, Precompile, PrecompileError, PrecompileFn, PrecompileId,
    PrecompileSpecId, Precompiles,
};

/// SCI shim newtype mirroring revm 38's `PrecompileOutput` shape.
///
/// v1.7.1 keychain source reads / writes the v38 fields directly
/// (`state_gas_used`, `reservoir`, `status`). To keep that source verbatim, the
/// shim mirrors those fields here. SCI's revm 34 stack has no concept of state
/// gas or reservoir, so the additional fields are kept at `0` and discarded
/// by [`to_revm34`] at the EVM-factory boundary.
#[derive(Clone, Debug)]
pub struct PrecompileOutput {
    /// Gas used by the precompile.
    pub gas_used: u64,
    /// Gas refunded by the precompile.
    pub gas_refunded: i64,
    /// Output bytes.
    pub bytes: Bytes,
    /// State-gas consumed by the precompile (v38 EIP-8037 / TIP-1016 accounting).
    /// Always `0` in SCI's revm 34 stack — the field is present only so that
    /// verbatim Tempo source like `output.state_gas_used = storage.state_gas_used();`
    /// compiles unmodified.
    pub state_gas_used: u64,
    /// Remaining state-gas reservoir (v38 EIP-8037). Always `0` for the same
    /// reason as [`Self::state_gas_used`].
    pub reservoir: u64,
    /// Execution outcome (success / revert / halt-with-reason). Replaces
    /// revm 34's `reverted: bool` flag; [`to_revm34`] folds back the `Halt`
    /// variant into `Err(PrecompileError::*)`.
    pub status: ExecutionStatus,
}

/// v38-style execution-status enum, exposed as [`PrecompileOutput::status`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ExecutionStatus {
    /// Call succeeded; `bytes` carries the ABI-encoded return value.
    #[default]
    Success,
    /// Call reverted with ABI-encoded error bytes.
    Revert,
    /// Call halted (OOG or fatal). At the [`to_revm34`] boundary this folds
    /// into `Err(PrecompileError::OutOfGas)` / `Err(PrecompileError::Other)`
    /// so revm 34 sees the same failure semantics as the v1.6 OOG path.
    Halt(PrecompileHalt),
}

impl ExecutionStatus {
    /// Returns `true` if the call succeeded.
    pub const fn is_success(&self) -> bool {
        matches!(self, Self::Success)
    }
    /// Returns `true` if the call reverted with ABI-encoded data.
    pub const fn is_revert(&self) -> bool {
        matches!(self, Self::Revert)
    }
    /// Returns `true` if the call halted (OOG / fatal).
    pub const fn is_halt(&self) -> bool {
        matches!(self, Self::Halt(_))
    }
}

/// v38-style halt-reason enum. SCI keychain only ever constructs two variants
/// (see `sci/crates/precompiles/src/error.rs` and `storage/thread_local.rs`).
/// Add more variants here only when upstream Tempo adds usage; the conversion
/// in [`to_revm34`] must be extended in lockstep.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrecompileHalt {
    /// Precompile ran out of gas.
    OutOfGas,
    /// Fatal / uncategorised halt; carries an explanatory message.
    Other(String),
}

/// v38-style result alias. Identical to revm 34's
/// `revm::precompile::PrecompileResult` in shape, but parameterised over the
/// shim's [`PrecompileOutput`] newtype.
pub type PrecompileResult = Result<PrecompileOutput, PrecompileError>;

impl PrecompileOutput {
    /// v38-shape `new(gas, bytes, reservoir)` constructor. The reservoir arg
    /// is kept for source compatibility but ignored.
    pub fn new(gas_used: u64, bytes: Bytes, _reservoir: u64) -> Self {
        Self {
            gas_used,
            gas_refunded: 0,
            bytes,
            state_gas_used: 0,
            reservoir: 0,
            status: ExecutionStatus::Success,
        }
    }

    /// v38-shape revert constructor: `(gas, bytes, reservoir)`.
    pub fn revert(gas_used: u64, bytes: Bytes, _reservoir: u64) -> Self {
        Self {
            gas_used,
            gas_refunded: 0,
            bytes,
            state_gas_used: 0,
            reservoir: 0,
            status: ExecutionStatus::Revert,
        }
    }

    /// v38-shape halt constructor: `(halt, reservoir)`. The reservoir arg is
    /// kept for source compatibility but ignored.
    pub fn halt(halt: PrecompileHalt, _reservoir: u64) -> Self {
        Self {
            gas_used: 0,
            gas_refunded: 0,
            bytes: Bytes::new(),
            state_gas_used: 0,
            reservoir: 0,
            status: ExecutionStatus::Halt(halt),
        }
    }

    /// Convenience accessor mirroring v1.7.1 source idioms (`output.is_revert()`).
    pub const fn is_revert(&self) -> bool {
        self.status.is_revert()
    }

    /// Convenience accessor mirroring v1.7.1 source idioms (`output.is_success()`).
    pub const fn is_success(&self) -> bool {
        self.status.is_success()
    }
}

/// Convert a shim [`PrecompileResult`] back into revm 34's native
/// `revm::precompile::PrecompileResult`. Invoked at the `DynPrecompile`
/// boundary inside `sci-precompiles::install`.
///
/// - `Err(PrecompileError)` flows through unchanged.
/// - `Ok` with `status == Halt(OutOfGas)` becomes
///   `Err(PrecompileError::OutOfGas)`.
/// - `Ok` with `status == Halt(Other(msg))` becomes
///   `Err(PrecompileError::Other(msg.into()))`.
/// - `Ok` with `status == Revert` or `Success` preserves all four revm 34
///   fields (`gas_used`, `gas_refunded`, `bytes`, `reverted`) verbatim.
pub fn to_revm34(out: PrecompileResult) -> revm::precompile::PrecompileResult {
    match out {
        Err(e) => Err(e),
        Ok(po) => match po.status {
            ExecutionStatus::Halt(PrecompileHalt::OutOfGas) => Err(PrecompileError::OutOfGas),
            ExecutionStatus::Halt(PrecompileHalt::Other(msg)) => {
                Err(PrecompileError::Other(msg.into()))
            }
            ExecutionStatus::Revert => Ok(revm::precompile::PrecompileOutput {
                gas_used: po.gas_used,
                gas_refunded: po.gas_refunded,
                bytes: po.bytes,
                reverted: true,
            }),
            ExecutionStatus::Success => Ok(revm::precompile::PrecompileOutput {
                gas_used: po.gas_used,
                gas_refunded: po.gas_refunded,
                bytes: po.bytes,
                reverted: false,
            }),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::Bytes;

    #[test]
    fn new_constructor_ignores_reservoir() {
        let out = PrecompileOutput::new(100, Bytes::from_static(&[1, 2, 3]), 999);
        assert_eq!(out.gas_used, 100);
        assert_eq!(out.bytes.as_ref(), &[1, 2, 3]);
        assert_eq!(out.reservoir, 0);
        assert!(out.is_success());
    }

    #[test]
    fn revert_constructor_sets_status() {
        let out = PrecompileOutput::revert(50, Bytes::from_static(&[9]), 0);
        assert!(out.is_revert());
        assert!(!out.is_success());
        assert_eq!(out.gas_used, 50);
    }

    #[test]
    fn halt_constructor_carries_reason() {
        let out = PrecompileOutput::halt(PrecompileHalt::OutOfGas, 0);
        assert!(out.status.is_halt());
        assert!(matches!(out.status, ExecutionStatus::Halt(PrecompileHalt::OutOfGas)));
    }

    #[test]
    fn to_revm34_oog_halt_becomes_err() {
        let shim = Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, 0));
        let revm = to_revm34(shim);
        assert!(matches!(revm, Err(PrecompileError::OutOfGas)));
    }

    #[test]
    fn to_revm34_other_halt_becomes_err_other() {
        let shim = Ok(PrecompileOutput::halt(
            PrecompileHalt::Other("boom".to_string()),
            0,
        ));
        let revm = to_revm34(shim);
        match revm {
            Err(PrecompileError::Other(msg)) => assert_eq!(msg, "boom"),
            other => panic!("expected Other halt to become Err::Other, got {other:?}"),
        }
    }

    #[test]
    fn to_revm34_revert_preserves_bytes_and_gas() {
        let shim = Ok(PrecompileOutput::revert(
            42,
            Bytes::from_static(&[0xDE, 0xAD]),
            0,
        ));
        let revm = to_revm34(shim).unwrap();
        assert!(revm.reverted);
        assert_eq!(revm.gas_used, 42);
        assert_eq!(revm.bytes.as_ref(), &[0xDE, 0xAD]);
    }

    #[test]
    fn to_revm34_success_preserves_bytes_and_gas() {
        let shim = Ok(PrecompileOutput::new(
            7,
            Bytes::from_static(&[0xBE, 0xEF]),
            0,
        ));
        let revm = to_revm34(shim).unwrap();
        assert!(!revm.reverted);
        assert_eq!(revm.gas_used, 7);
        assert_eq!(revm.bytes.as_ref(), &[0xBE, 0xEF]);
    }
}
