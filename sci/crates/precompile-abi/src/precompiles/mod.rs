//! SCI Chain precompile predeploy addresses and ABI bindings.

mod account_keychain;
pub use account_keychain::{
    AccountKeychainError, AccountKeychainEvent, IAccountKeychain, authorizeKeyCall,
    authorizeKeyWithWitnessCall, getAllowedCallsReturn, getRemainingLimitReturn,
    getRemainingLimitWithPeriodCall, legacyAuthorizeKeyCall,
};

mod common_errors;
pub use common_errors::UnknownFunctionSelector;

mod tip20;
pub use tip20::ITIP20;

mod sci_agent_state;
use alloy_primitives::{Address, address};
pub use sci_agent_state::{
    AGENT_CIRCUIT_BREAKER_ADDRESS, ISciAgentState, SCI_AGENT_STATE_ADDRESS, SciAgentStateError,
    SciAgentStateEvent, isTrippedCall, tripKeyCall, untripKeyCall,
};

/// AccountKeychain precompile address.
pub const ACCOUNT_KEYCHAIN_ADDRESS: Address =
    address!("0xAAAAAAAA00000000000000000000000000000000");

/// Default fee token address (placeholder; SCI Chain uses standard ERC-20).
pub const DEFAULT_FEE_TOKEN: Address = address!("0x20C0000000000000000000000000000000000000");
