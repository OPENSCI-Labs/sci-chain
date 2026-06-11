//! ABI for the `SciAgentState` precompile.
//!
//! `SciAgentState` holds SCI-only protocol state that doesn't belong inside ported Tempo
//! source. Currently it stores `CircuitBreaker` trip flags per session key. The Solidity
//! `AgentCircuitBreaker.sol` predeploy at `AGENT_CIRCUIT_BREAKER_ADDRESS` (Heath's lane)
//! is a thin façade — admin access control + event emission — that forwards `trip` /
//! `untrip` here. Read-side consumers (the pre-execution hook, the delegator) call
//! `isTripped` directly on this precompile.

use alloy_primitives::{Address, address};

pub use ISciAgentState::{
    ISciAgentStateErrors as SciAgentStateError, ISciAgentStateEvents as SciAgentStateEvent,
    isTrippedCall, tripKeyCall, untripKeyCall,
};

crate::sol! {
    /// SCI agent state precompile interface.
    ///
    /// Mutators (`tripKey`, `untripKey`) are restricted to
    /// `msg.sender == AGENT_CIRCUIT_BREAKER_ADDRESS`. The view `isTripped` is callable by
    /// anyone (the hook, the Solidity delegator, off-chain monitors).
    #[derive(Debug, PartialEq, Eq)]
    #[sol(abi)]
    interface ISciAgentState {
        /// Sets `tripped[sessionKey] = true`. Restricted to `AGENT_CIRCUIT_BREAKER_ADDRESS`.
        function tripKey(address sessionKey) external;
        /// Sets `tripped[sessionKey] = false`. Restricted to `AGENT_CIRCUIT_BREAKER_ADDRESS`.
        function untripKey(address sessionKey) external;
        /// Returns the trip flag for `sessionKey`.
        function isTripped(address sessionKey) external view returns (bool);

        /// Emitted when an authorized caller flips the trip flag.
        event TripStateUpdate(address indexed sessionKey, bool isTripped);

        /// Returned when a non-`AGENT_CIRCUIT_BREAKER_ADDRESS` caller invokes a mutator.
        /// Named distinctly from `AccountKeychainError::UnauthorizedCaller` so the two have
        /// non-colliding 4-byte selectors and can be decoded unambiguously by clients.
        error Unauthorized();

        /// Returned (by the pre-execution hook) when an agent tx's session key has been
        /// tripped by the circuit breaker. A business error — not a system fault — so the
        /// block builder skips the tx instead of treating it as a payload-build failure.
        error KeyTripped(address sessionKey);
    }
}

impl SciAgentStateError {
    /// Helper constructing the `Unauthorized` variant.
    pub const fn unauthorized_caller() -> Self {
        Self::Unauthorized(ISciAgentState::Unauthorized {})
    }

    /// Helper constructing the `KeyTripped` variant.
    pub const fn key_tripped(session_key: Address) -> Self {
        Self::KeyTripped(ISciAgentState::KeyTripped { sessionKey: session_key })
    }
}

/// `SciAgentState` precompile address — sibling to `ACCOUNT_KEYCHAIN_ADDRESS` (`...0000`).
pub const SCI_AGENT_STATE_ADDRESS: Address =
    address!("0xAAAAAAAA00000000000000000000000000000001");

/// Address of the Solidity `AgentCircuitBreaker.sol` predeploy. The only address allowed
/// to call `tripKey` / `untripKey` on `SciAgentState`.
pub const AGENT_CIRCUIT_BREAKER_ADDRESS: Address =
    address!("0xBBBBBBBB00000000000000000000000000000003");
