//! SCI Chain EVM precompile implementations.
//!
//! Currently exposes the [`AccountKeychain`] precompile (at
//! [`tempo_contracts::precompiles::ACCOUNT_KEYCHAIN_ADDRESS`]), which manages session keys and
//! per-token spending limits at the protocol level.

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
pub use handler::{HookOutcome, apply_post_execution_deductions, run_pre_execution_hook};

#[cfg(any(test, feature = "test-utils"))]
pub mod test_util;

pub use tempo_contracts::precompiles::{
    ACCOUNT_KEYCHAIN_ADDRESS, AGENT_CIRCUIT_BREAKER_ADDRESS, SCI_AGENT_STATE_ADDRESS,
};

use crate::storage::StorageCtx;
use alloy_evm::precompiles::{DynPrecompile, PrecompilesMap};
use alloy_primitives::{Address, Bytes};
use alloy_sol_types::{SolCall, SolError, sol};
use revm::{
    context::CfgEnv,
    context_interface::cfg::GasParams,
    handler::EthPrecompiles,
    precompile::{PrecompileError, PrecompileId, PrecompileOutput, PrecompileResult},
    primitives::hardfork::SpecId,
};

pub use crate::error::PrecompileHalt;

/// Input per word cost. Covers ABI decoding and cloning of input into call data.
pub const INPUT_PER_WORD_COST: u64 = 6;

/// Gas cost for `ecrecover` signature verification (used by KeyAuthorization and Permit).
pub const ECRECOVER_GAS: u64 = 3_000;

/// Returns the gas cost for decoding calldata of the given length, rounded up to word boundaries.
#[inline]
pub fn input_cost(calldata_len: usize) -> u64 {
    calldata_len
        .div_ceil(32)
        .saturating_mul(INPUT_PER_WORD_COST as usize) as u64
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
/// rejects delegatecalls, sets up a storage context, then invokes the body.
macro_rules! sci_precompile {
    ($id:expr, $spec:expr, $gas_params:expr, |$input:ident| $impl:expr) => {{
        let spec = $spec;
        let gas_params = $gas_params;
        DynPrecompile::new_stateful(PrecompileId::Custom($id.into()), move |$input| {
            if !$input.is_direct_call() {
                return Ok(PrecompileOutput::new_reverted(
                    0,
                    DelegateCallNotAllowed {}.abi_encode().into(),
                ));
            }
            let mut storage = crate::storage::evm::EvmPrecompileStorageProvider::new(
                $input.internals,
                $input.gas,
                0,
                spec,
                $input.is_static,
                gas_params.clone(),
            );
            crate::storage::StorageCtx::enter(&mut storage, || {
                $impl.call($input.data, $input.caller)
            })
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
        if *address == ACCOUNT_KEYCHAIN_ADDRESS {
            Some(AccountKeychain::create_precompile(
                TempoHardfork::T3,
                gas_params.clone(),
            ))
        } else if *address == SCI_AGENT_STATE_ADDRESS {
            Some(SciAgentState::create_precompile(
                TempoHardfork::T3,
                gas_params.clone(),
            ))
        } else {
            None
        }
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
pub(crate) fn view<T: SolCall>(call: T, f: impl FnOnce(T) -> Result<T::Return>) -> PrecompileResult {
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
        return Ok(PrecompileOutput::new_reverted(
            0,
            StaticCallNotAllowed {}.abi_encode().into(),
        ));
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
        return Ok(PrecompileOutput::new_reverted(
            0,
            StaticCallNotAllowed {}.abi_encode().into(),
        ));
    }
    f(sender, call).into_precompile_result(0, 0, |()| Bytes::new())
}

/// Deducts the calldata input cost, returning an OOG halt result if insufficient gas.
#[inline]
pub(crate) fn charge_input_cost(
    storage: &mut StorageCtx,
    calldata: &[u8],
) -> Option<PrecompileResult> {
    if storage.deduct_gas(input_cost(calldata.len())).is_err() {
        return Some(Err(storage.halt_output(PrecompileHalt::OutOfGas)));
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
        if self.hardfork <= active {
            self.dropped
        } else {
            self.added
        }
        .contains(&selector)
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
            return Err(storage.halt_output(PrecompileHalt::Other(
                "Invalid input: missing function selector".into(),
            )));
        }
    }

    let selector: [u8; 4] = calldata[..4].try_into().expect("calldata len >= 4");
    if hardforks.iter().any(|s| s.rejects(selector, storage.spec())) {
        return storage.error_result(error::TempoPrecompileError::UnknownFunctionSelector(selector));
    }

    match decode(calldata) {
        Ok(call) => f(call).map(|mut res| {
            res.gas_used = storage.gas_used();
            res
        }),
        Err(alloy_sol_types::Error::UnknownSelector { selector, .. }) => storage
            .error_result(error::TempoPrecompileError::UnknownFunctionSelector(*selector)),
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
            assert!(out.reverted, "expected reverted output, got: {out:?}");
            let decoded = E::abi_decode(&out.bytes).unwrap();
            assert_eq!(decoded, expected_error);
        }
        Err(other) => panic!("expected reverted output, got: {other:?}"),
    }
}
