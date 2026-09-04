//! SCI Chain EVM precompile implementations.
//!
//! Currently exposes the [`AccountKeychain`] precompile (at
//! [`tempo_contracts::precompiles::ACCOUNT_KEYCHAIN_ADDRESS`]), which manages session keys and
//! per-token spending limits at the protocol level, and the [`SciAgentState`] precompile
//! (SCI-only CircuitBreaker trip state, at
//! [`tempo_contracts::precompiles::SCI_AGENT_STATE_ADDRESS`]).
//!
//! ## revm shim
//!
//! This crate depends on `revm` via the `sci-revm-shim` Cargo `package = ...` rename
//! (see `Cargo.toml`). Every `use revm::*` inside this crate resolves through the shim,
//! which mirrors revm 38's `PrecompileOutput` / `PrecompileHalt` / state-gas API surface
//! on top of Base v0.9's revm 34. At the [`DynPrecompile`] boundary the shim's
//! [`revm::precompile::to_revm34`] helper folds shim-shaped outputs back into real revm 34
//! `PrecompileResult` values (halt → `Err(PrecompileError::*)`, success/revert preserve
//! gas + bytes). Tempo verbatim source can therefore be `cp`'d verbatim without per-file
//! patches.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

/// SCI-facing re-export of [`tempo_chainspec::hardfork::TempoHardfork`] under the SCI name.
/// The keychain source uses `TempoHardfork` internally (for verbatim Tempo compatibility);
/// SCI-facing API surfaces `SciHardfork` here.
pub use tempo_chainspec::hardfork::{SciHardfork, TempoHardfork};

pub mod error;
pub use error::{IntoPrecompileResult, Result, SciPrecompileError, TempoPrecompileError};

pub mod storage;

pub mod account_keychain;
pub use account_keychain::AccountKeychain;

pub mod sci_agent_state;
pub use sci_agent_state::SciAgentState;

pub mod handler;
pub use handler::{
    AaCall, HookOutcome, apply_aa_post_execution_deductions, keychain_tx_origin,
    run_aa_keychain_hook, set_keychain_tx_origin,
};

#[cfg(any(test, feature = "test-utils"))]
pub mod test_util;

use alloy_evm::precompiles::{DynPrecompile, PrecompilesMap};
use alloy_primitives::{Address, Bytes};
use alloy_sol_types::{SolCall, SolError, sol};
use revm::{
    context::CfgEnv,
    context_interface::cfg::GasParams,
    handler::EthPrecompiles,
    precompile::{PrecompileHalt, PrecompileId, PrecompileOutput, PrecompileResult, to_revm34},
    primitives::hardfork::SpecId,
};
pub use tempo_contracts::precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, AGENT_CIRCUIT_BREAKER_ADDRESS, SCI_AGENT_STATE_ADDRESS,
};

use crate::storage::StorageCtx;

/// Input per word cost. Covers ABI decoding and cloning of input into call data.
pub const INPUT_PER_WORD_COST: u64 = 6;

/// Gas cost for `ecrecover` signature verification (used by KeyAuthorization and Permit).
pub const ECRECOVER_GAS: u64 = 3_000;

/// Returns the gas cost for decoding calldata of the given length, rounded up to word boundaries.
#[inline]
pub fn input_cost(calldata_len: usize) -> u64 {
    calldata_len.div_ceil(32).saturating_mul(INPUT_PER_WORD_COST as usize) as u64
}

/// Trait implemented by all SCI Chain precompile contract types.
pub trait Precompile {
    /// Dispatches an EVM call to this precompile.
    fn call(&mut self, calldata: &[u8], msg_sender: Address) -> PrecompileResult;
}

sol! {
    /// Returned when a precompile is invoked via DELEGATECALL.
    error DelegateCallNotAllowed();
    /// Returned when a state-mutating precompile method is invoked via STATICCALL.
    error StaticCallNotAllowed();
}

/// Wraps an inner [`Precompile`] implementation in a stateful [`DynPrecompile`]:
/// rejects delegatecalls, sets up a storage context, runs the body, and folds the
/// shim-shaped result back to revm 34's `PrecompileResult` via [`to_revm34`].
///
/// The `reservoir` argument threaded into the shim's `PrecompileOutput::*`
/// constructors is always `0` and the `amsterdam_eip8037_enabled` flag passed to
/// the storage provider is always `false` — SCI does not adopt EIP-8037 /
/// TIP-1016 state-gas accounting (see `sci-revm-shim` crate docs).
macro_rules! sci_precompile {
    ($id:expr, $spec:expr, $gas_params:expr, |$input:ident| $impl:expr) => {{
        let spec = $spec;
        let gas_params = $gas_params;
        DynPrecompile::new_stateful(PrecompileId::Custom($id.into()), move |$input| {
            let result: PrecompileResult = (|| -> PrecompileResult {
                if !$input.is_direct_call() {
                    return Ok(PrecompileOutput::revert(
                        0,
                        DelegateCallNotAllowed {}.abi_encode().into(),
                        0,
                    ));
                }
                let mut storage = crate::storage::evm::EvmPrecompileStorageProvider::new(
                    $input.internals,
                    $input.gas,
                    0,
                    spec,
                    false,
                    $input.is_static,
                    gas_params.clone(),
                );
                crate::storage::StorageCtx::enter(&mut storage, || {
                    $impl.call($input.data, $input.caller)
                })
            })();
            to_revm34(result)
        })
    }};
}

impl AccountKeychain {
    /// Creates the EVM [`DynPrecompile`] for this type.
    ///
    /// `spec` is the SCI hardfork level the precompile gates its selectors on; for SCI Chain
    /// at launch this is [`TempoHardfork::T3`] (full feature set).
    pub fn create_precompile(spec: TempoHardfork, gas_params: GasParams) -> DynPrecompile {
        sci_precompile!("AccountKeychain", spec, gas_params, |input| { Self::new() })
    }
}

impl SciAgentState {
    /// Creates the EVM [`DynPrecompile`] for this type.
    pub fn create_precompile(spec: TempoHardfork, gas_params: GasParams) -> DynPrecompile {
        sci_precompile!("SciAgentState", spec, gas_params, |input| { Self::new() })
    }
}

/// Returns `true` iff `address` hosts an SCI precompile ([`ACCOUNT_KEYCHAIN_ADDRESS`]
/// or [`SCI_AGENT_STATE_ADDRESS`]).
///
/// Every precompile provider in the system — the EL's [`PrecompilesMap`] lookup
/// installed by [`install`] AND the proof client's zkVM provider — must agree on this
/// set, or the sequencer and the verifier diverge on whether a call to these addresses
/// executes a precompile or the `0xef` genesis placeholder code.
#[inline]
pub fn is_sci_precompile_address(address: &Address) -> bool {
    *address == ACCOUNT_KEYCHAIN_ADDRESS || *address == SCI_AGENT_STATE_ADDRESS
}

/// Resolves an SCI precompile for `address`, if any — the single source of truth used
/// by both the EL host integration ([`install`]) and the proof client's zkVM
/// precompile provider, so the two execute identical code at these addresses.
pub fn lookup_precompile(address: &Address, gas_params: &GasParams) -> Option<DynPrecompile> {
    if *address == ACCOUNT_KEYCHAIN_ADDRESS {
        Some(AccountKeychain::create_precompile(SCI_LAUNCH_HARDFORK, gas_params.clone()))
    } else if *address == SCI_AGENT_STATE_ADDRESS {
        Some(SciAgentState::create_precompile(SCI_LAUNCH_HARDFORK, gas_params.clone()))
    } else {
        None
    }
}

/// The Tempo-hardfork level SCI launches its precompiles at — the single switch that
/// turns hardfork-gated keychain features on/off chain-wide.
///
/// T5 (from Tempo v1.7.1) makes the TIP-1053 key authorization witness API
/// (`authorizeKey(_, _, _, witness)`, `burnKeyAuthorizationWitness`,
/// `isKeyAuthorizationWitnessBurned`) reachable; the selector schedule in
/// `account_keychain/dispatch.rs` still gates the T5 selectors behind `is_t5()`.
/// Both the [`install`] lookup and the pre-execution hook's storage provider
/// (`handler/hook.rs::enter_keychain_storage`) must consult the same value, or the
/// hook would read keychain state under different packing/gating rules than the
/// precompile writes it with.
pub const SCI_LAUNCH_HARDFORK: TempoHardfork = TempoHardfork::T5;

/// Installs SCI Chain's precompile lookup on top of an existing [`PrecompilesMap`].
///
/// Call this after constructing the base map from `EthPrecompiles` to add SCI precompiles.
/// Currently registers:
/// - [`AccountKeychain`] at [`ACCOUNT_KEYCHAIN_ADDRESS`] — session keys + spending limits.
/// - [`SciAgentState`] at [`SCI_AGENT_STATE_ADDRESS`] — SCI-only protocol state (CB flags).
///
/// Other addresses fall through to the underlying map.
pub fn install<Spec: Copy + 'static>(precompiles: &mut PrecompilesMap, cfg: &CfgEnv<Spec>) {
    let gas_params = cfg.gas_params.clone();
    precompiles.set_precompile_lookup(move |address: &Address| -> Option<DynPrecompile> {
        lookup_precompile(address, &gas_params)
    });
}

/// Returns a fresh [`PrecompilesMap`] containing the Prague Ethereum precompiles plus
/// SCI Chain's custom precompiles.
///
/// Provided as a convenience for tests and standalone EVM construction; production
/// integrations should call [`install`] on the host's existing [`PrecompilesMap`] instead.
pub fn sci_precompiles<Spec: Copy + 'static>(cfg: &CfgEnv<Spec>) -> PrecompilesMap {
    let mut precompiles =
        PrecompilesMap::from_static(EthPrecompiles::new(SpecId::PRAGUE).precompiles);
    install(&mut precompiles, cfg);
    precompiles
}

/// Dispatches a parameterless view call, encoding the return via `T`.
#[inline]
pub(crate) fn metadata<T: SolCall>(f: impl FnOnce() -> Result<T::Return>) -> PrecompileResult {
    f().into_precompile_result(0, 0, |ret| T::abi_encode_returns(&ret).into())
}

/// Dispatches a read-only call with decoded arguments, encoding the return via `T`.
#[inline]
pub(crate) fn view<T: SolCall>(
    call: T,
    f: impl FnOnce(T) -> Result<T::Return>,
) -> PrecompileResult {
    f(call).into_precompile_result(0, 0, |ret| T::abi_encode_returns(&ret).into())
}

/// Dispatches a state-mutating call that returns ABI-encoded data.
#[inline]
pub(crate) fn mutate<T: SolCall>(
    call: T,
    sender: Address,
    f: impl FnOnce(Address, T) -> Result<T::Return>,
) -> PrecompileResult {
    if StorageCtx.is_static() {
        return Ok(PrecompileOutput::revert(0, StaticCallNotAllowed {}.abi_encode().into(), 0));
    }
    f(sender, call).into_precompile_result(0, 0, |ret| T::abi_encode_returns(&ret).into())
}

/// Dispatches a state-mutating call that returns no data.
#[inline]
pub(crate) fn mutate_void<T: SolCall>(
    call: T,
    sender: Address,
    f: impl FnOnce(Address, T) -> Result<()>,
) -> PrecompileResult {
    if StorageCtx.is_static() {
        return Ok(PrecompileOutput::revert(0, StaticCallNotAllowed {}.abi_encode().into(), 0));
    }
    f(sender, call).into_precompile_result(0, 0, |()| Bytes::new())
}

/// Deducts the calldata input cost, returning an OOG halt result if insufficient gas.
///
/// The shim's `halt_output(PrecompileHalt::OutOfGas)` returns an `Ok(PrecompileOutput)`
/// carrying the halt status; the boundary [`to_revm34`] step folds it back to
/// `Err(PrecompileError::OutOfGas)` for revm 34. This matches v1.7.1 idiom even though
/// SCI's underlying revm 34 still surfaces OOG as `Err`.
#[inline]
pub(crate) fn charge_input_cost(
    storage: &mut StorageCtx,
    calldata: &[u8],
) -> Option<PrecompileResult> {
    if storage.deduct_gas(input_cost(calldata.len())).is_err() {
        return Some(Ok(storage.halt_output(PrecompileHalt::OutOfGas)));
    }
    None
}

/// A selector schedule at a given hardfork boundary.
#[derive(Clone, Copy, Debug, Default)]
pub struct SelectorSchedule<'a> {
    hardfork: TempoHardfork,
    added: &'a [[u8; 4]],
    dropped: &'a [[u8; 4]],
}

impl<'a> SelectorSchedule<'a> {
    /// Creates a new schedule anchored at `hardfork` with no selectors registered yet.
    pub const fn new(hardfork: TempoHardfork) -> Self {
        Self { hardfork, added: &[], dropped: &[] }
    }

    /// Registers selectors that are introduced at this hardfork boundary.
    pub const fn with_added(mut self, selectors: &'a [[u8; 4]]) -> Self {
        self.added = selectors;
        self
    }

    /// Registers selectors that are removed at this hardfork boundary.
    pub const fn with_dropped(mut self, selectors: &'a [[u8; 4]]) -> Self {
        self.dropped = selectors;
        self
    }

    /// Returns `true` if this schedule gates out `selector` under the `active` hardfork.
    #[inline]
    fn rejects(self, selector: [u8; 4], active: TempoHardfork) -> bool {
        if self.hardfork <= active { self.dropped } else { self.added }.contains(&selector)
    }
}

/// Applies hardfork selector schedules, decodes calldata via `decode`, then dispatches to `f`.
#[inline]
pub(crate) fn dispatch_call<T>(
    calldata: &[u8],
    hardforks: &[SelectorSchedule<'_>],
    decode: impl FnOnce(&[u8]) -> core::result::Result<T, alloy_sol_types::Error>,
    f: impl FnOnce(T) -> PrecompileResult,
) -> PrecompileResult {
    let storage = StorageCtx::default();

    if calldata.len() < 4 {
        if storage.spec().is_t1() {
            return Ok(storage.revert_output(Bytes::new()));
        } else {
            return Ok(storage.halt_output(PrecompileHalt::Other(
                "Invalid input: missing function selector".into(),
            )));
        }
    }

    let selector: [u8; 4] = calldata[..4].try_into().expect("calldata len >= 4");
    if hardforks.iter().any(|s| s.rejects(selector, storage.spec())) {
        return storage
            .error_result(error::TempoPrecompileError::UnknownFunctionSelector(selector));
    }

    match decode(calldata) {
        Ok(call) => f(call).map(|mut res| {
            res.gas_used = storage.gas_used();
            res
        }),
        Err(alloy_sol_types::Error::UnknownSelector { selector, .. }) => {
            storage.error_result(error::TempoPrecompileError::UnknownFunctionSelector(*selector))
        }
        Err(_) => Ok(storage.revert_output(Bytes::new())),
    }
}

/// Asserts that `result` is a reverted output whose bytes decode to `expected_error`.
#[cfg(test)]
pub fn expect_precompile_revert<E>(result: &PrecompileResult, expected_error: E)
where
    E: alloy_sol_types::SolInterface + PartialEq + core::fmt::Debug,
{
    match result {
        Ok(out) => {
            assert!(out.is_revert(), "expected reverted output, got: {out:?}");
            let decoded = E::abi_decode(&out.bytes).unwrap();
            assert_eq!(decoded, expected_error);
        }
        Err(other) => panic!("expected reverted output, got: {other:?}"),
    }
}
