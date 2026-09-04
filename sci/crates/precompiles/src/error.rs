//! Unified error handling for SCI Chain precompiles.
//!
//! The primary type is [`TempoPrecompileError`] (named to match the verbatim Tempo source
//! that imports `crate::error::TempoPrecompileError` via the `Storable` proc-macro
//! expansion). [`SciPrecompileError`] is provided as a type alias for SCI-facing
//! consumers — both names refer to the same type.

use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
};

use alloy_evm::EvmInternalsError;
use alloy_primitives::{Bytes, Selector, U256};
use alloy_sol_types::{Panic, PanicKind, SolError, SolInterface};
use revm::{
    context::journaled_state::JournalLoadError,
    precompile::{PrecompileError, PrecompileHalt, PrecompileOutput, PrecompileResult},
};
use tempo_contracts::precompiles::{
    AccountKeychainError, SciAgentStateError, UnknownFunctionSelector,
};

/// Top-level error type for all SCI Chain precompile operations.
///
/// Named `TempoPrecompileError` so that proc-macro-emitted code (which references
/// `crate::error::TempoPrecompileError` verbatim from the Tempo source) resolves
/// here. [`SciPrecompileError`] is an alias on the same type for SCI-facing API.
#[derive(
    Debug, Clone, PartialEq, Eq, thiserror::Error, derive_more::From, derive_more::TryInto,
)]
pub enum TempoPrecompileError {
    /// EVM panic (i.e. arithmetic under/overflow, out-of-bounds access).
    #[error("Panic({0:?})")]
    Panic(PanicKind),

    /// Error from the account keychain precompile.
    #[error("Account keychain error: {0:?}")]
    AccountKeychainError(AccountKeychainError),

    /// Error from the SCI agent state precompile.
    #[error("SCI agent state error: {0:?}")]
    SciAgentStateError(SciAgentStateError),

    /// Gas limit exceeded during precompile execution.
    #[error("Gas limit exceeded")]
    OutOfGas,

    /// The calldata's 4-byte selector does not match any known precompile function.
    #[error("Unknown function selector: {0:?}")]
    UnknownFunctionSelector([u8; 4]),

    /// Unrecoverable internal error (e.g. database failure).
    #[error("Fatal precompile error: {0:?}")]
    #[from(skip)]
    Fatal(String),
}

/// SCI-facing alias for [`TempoPrecompileError`]. Both names refer to the same type.
pub use TempoPrecompileError as SciPrecompileError;

impl From<EvmInternalsError> for TempoPrecompileError {
    fn from(value: EvmInternalsError) -> Self {
        match value {
            EvmInternalsError::Database(e) => Self::Fatal(e.to_string()),
        }
    }
}

impl From<JournalLoadError<EvmInternalsError>> for TempoPrecompileError {
    fn from(value: JournalLoadError<EvmInternalsError>) -> Self {
        match value {
            JournalLoadError::DBError(e) => Self::from(e),
            JournalLoadError::ColdLoadSkipped => Self::OutOfGas,
        }
    }
}

impl From<JournalLoadError<revm::context::ErasedError>> for TempoPrecompileError {
    fn from(value: JournalLoadError<revm::context::ErasedError>) -> Self {
        match value {
            JournalLoadError::DBError(e) => Self::Fatal(e.to_string()),
            JournalLoadError::ColdLoadSkipped => Self::OutOfGas,
        }
    }
}

/// Result type alias for SCI Chain precompile operations.
pub type Result<T> = std::result::Result<T, TempoPrecompileError>;

impl TempoPrecompileError {
    /// Returns true if this error represents a system-level failure that must be propagated
    /// rather than swallowed, because state may be inconsistent.
    pub const fn is_system_error(&self) -> bool {
        match self {
            Self::OutOfGas | Self::Fatal(_) | Self::Panic(_) => true,
            Self::AccountKeychainError(_)
            | Self::SciAgentStateError(_)
            | Self::UnknownFunctionSelector(_) => false,
        }
    }

    /// Creates an arithmetic under/overflow panic error.
    pub const fn under_overflow() -> Self {
        Self::Panic(PanicKind::UnderOverflow)
    }

    /// Creates an enum conversion error panic (Solidity Panic `0x21`).
    pub const fn enum_conversion_error() -> Self {
        Self::Panic(PanicKind::EnumConversionError)
    }

    /// Creates an array out-of-bounds panic error.
    pub const fn array_oob() -> Self {
        Self::Panic(PanicKind::ArrayOutOfBounds)
    }

    /// ABI-encodes this error and wraps it as a reverted [`PrecompileResult`].
    ///
    /// `reservoir` is accepted for source-compatibility with Tempo v1.7.1 (which
    /// threads EIP-8037 reservoir state through every precompile path). SCI's
    /// revm 34 stack has no reservoir, so the value is passed through to the
    /// shim's `PrecompileOutput::{new,revert,halt}` constructors and discarded
    /// there.
    pub fn into_precompile_result(self, gas: u64, reservoir: u64) -> PrecompileResult {
        let bytes: Bytes = match self {
            Self::AccountKeychainError(e) => e.abi_encode().into(),
            Self::SciAgentStateError(e) => e.abi_encode().into(),
            Self::Panic(kind) => {
                let panic = Panic { code: U256::from(kind as u32) };
                panic.abi_encode().into()
            }
            Self::OutOfGas => {
                // v1.7.1 idiom: signal OOG via `PrecompileOutput::halt(...)`. Our
                // shim folds this back into `Err(PrecompileError::OutOfGas)` at
                // the `to_revm34` boundary inside `install()`.
                return Ok(PrecompileOutput::halt(PrecompileHalt::OutOfGas, reservoir));
            }
            Self::UnknownFunctionSelector(selector) => {
                UnknownFunctionSelector { selector: selector.into() }.abi_encode().into()
            }
            Self::Fatal(msg) => {
                return Err(PrecompileError::Fatal(msg));
            }
        };
        Ok(PrecompileOutput::revert(gas, bytes, reservoir))
    }
}

/// Registers all ABI error selectors for a [`SolInterface`] type into the decoder registry.
pub fn add_errors_to_registry<T: SolInterface>(
    registry: &mut TempoPrecompileErrorRegistry,
    converter: impl Fn(T) -> TempoPrecompileError + 'static + Send + Sync,
) {
    let converter = Arc::new(converter);
    for selector in T::selectors() {
        let converter = Arc::clone(&converter);
        registry.insert(
            selector.into(),
            Box::new(move |data: &[u8]| {
                T::abi_decode(data).ok().map(|error| DecodedTempoPrecompileError {
                    error: converter(error),
                    revert_bytes: data,
                })
            }),
        );
    }
}

/// A decoded precompile error together with the raw revert bytes.
pub struct DecodedTempoPrecompileError<'a> {
    /// The decoded typed error.
    pub error: TempoPrecompileError,
    /// The original ABI-encoded revert bytes.
    pub revert_bytes: &'a [u8],
}

/// SCI-facing alias for [`DecodedTempoPrecompileError`].
pub use DecodedTempoPrecompileError as DecodedSciPrecompileError;

/// Maps ABI error selectors to their decoder functions.
pub type TempoPrecompileErrorRegistry = HashMap<
    Selector,
    Box<dyn for<'a> Fn(&'a [u8]) -> Option<DecodedTempoPrecompileError<'a>> + Send + Sync>,
>;

/// SCI-facing alias for [`TempoPrecompileErrorRegistry`].
pub type SciPrecompileErrorRegistry = TempoPrecompileErrorRegistry;

/// Builds a [`TempoPrecompileErrorRegistry`] mapping every known error selector to its decoder.
pub fn error_decoder_registry() -> TempoPrecompileErrorRegistry {
    let mut registry: TempoPrecompileErrorRegistry = HashMap::new();
    add_errors_to_registry(&mut registry, TempoPrecompileError::AccountKeychainError);
    add_errors_to_registry(&mut registry, TempoPrecompileError::SciAgentStateError);
    registry
}

/// Global lazily-initialized registry of all SCI precompile error decoders.
pub static ERROR_REGISTRY: LazyLock<TempoPrecompileErrorRegistry> =
    LazyLock::new(error_decoder_registry);

/// Decodes raw revert bytes into a typed [`DecodedTempoPrecompileError`] using the global
/// [`ERROR_REGISTRY`], returning `None` if the data is shorter than 4 bytes or the selector
/// is unrecognized.
pub fn decode_error(data: &[u8]) -> Option<DecodedTempoPrecompileError<'_>> {
    if data.len() < 4 {
        return None;
    }
    let selector: [u8; 4] = data[0..4].try_into().ok()?;
    ERROR_REGISTRY.get(&selector).and_then(|decoder| decoder(data))
}

/// Extension trait to convert `Result<T, TempoPrecompileError>` into a [`PrecompileResult`].
pub trait IntoPrecompileResult<T> {
    /// Converts `self` into a [`PrecompileResult`], using `encode_ok` for the success path.
    fn into_precompile_result(
        self,
        gas: u64,
        reservoir: u64,
        encode_ok: impl FnOnce(T) -> Bytes,
    ) -> PrecompileResult;
}

impl<T> IntoPrecompileResult<T> for Result<T> {
    fn into_precompile_result(
        self,
        gas: u64,
        reservoir: u64,
        encode_ok: impl FnOnce(T) -> Bytes,
    ) -> PrecompileResult {
        match self {
            Ok(res) => Ok(PrecompileOutput::new(gas, encode_ok(res), reservoir)),
            Err(err) => err.into_precompile_result(gas, reservoir),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_error_returns_some_for_valid_keychain_error() {
        let error = AccountKeychainError::unauthorized_caller();
        let encoded = error.abi_encode();

        let result = decode_error(&encoded);
        assert!(result.is_some(), "decode_error should return Some for a valid keychain error");

        let decoded = result.unwrap();
        assert!(matches!(
            decoded.error,
            TempoPrecompileError::AccountKeychainError(AccountKeychainError::UnauthorizedCaller(_))
        ));
    }

    #[test]
    fn test_decode_error_short_data_returns_none() {
        assert!(decode_error(&[]).is_none());
        assert!(decode_error(&[0x01, 0x02, 0x03]).is_none());
    }

    #[test]
    fn test_decode_error_unknown_selector_returns_none() {
        assert!(decode_error(&[0xff; 4]).is_none());
    }
}
