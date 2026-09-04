//! SCI-only agent state precompile.
//!
//! Holds per–session-key protocol state that doesn't belong inside ported Tempo source.
//! Currently: a `tripped: Mapping<Address, bool>` for the `CircuitBreaker`. Solidity
//! `AgentCircuitBreaker.sol` at [`AGENT_CIRCUIT_BREAKER_ADDRESS`] is the only address
//! allowed to mutate this state; reads are open.
//!
//! Putting this state here (rather than inside [`AccountKeychain`]) preserves CLAUDE.md
//! Rule #4 — Tempo source files stay verbatim. New SCI-only protocol state (attribution
//! counters, MPP session info, …) can be added as additional fields on this struct over
//! time without touching ported files.

pub mod dispatch;

use alloy_primitives::Address;
use tempo_contracts::precompiles::{
    AGENT_CIRCUIT_BREAKER_ADDRESS, ISciAgentState, SCI_AGENT_STATE_ADDRESS, SciAgentStateError,
    SciAgentStateEvent, isTrippedCall, tripKeyCall, untripKeyCall,
};
use tempo_precompiles_macros::contract;

use crate::{
    error::Result,
    storage::{Handler, Mapping},
};

#[contract(addr = SCI_AGENT_STATE_ADDRESS)]
pub struct SciAgentState {
    /// `tripped[sessionKey] = true` iff the session key is frozen by the CircuitBreaker.
    tripped: Mapping<Address, bool>,
}

impl SciAgentState {
    /// Initializes the precompile (writes a placeholder `0xef` bytecode to the storage address).
    pub fn initialize(&mut self) -> Result<()> {
        self.__initialize()
    }

    /// Sets `tripped[sessionKey] = true`. Authorized to [`AGENT_CIRCUIT_BREAKER_ADDRESS`] only.
    pub fn trip_key(&mut self, msg_sender: Address, call: tripKeyCall) -> Result<()> {
        if msg_sender != AGENT_CIRCUIT_BREAKER_ADDRESS {
            return Err(SciAgentStateError::unauthorized_caller().into());
        }
        self.tripped[call.sessionKey].write(true)?;
        self.emit_event(SciAgentStateEvent::TripStateUpdate(ISciAgentState::TripStateUpdate {
            sessionKey: call.sessionKey,
            isTripped: true,
        }))
    }

    /// Sets `tripped[sessionKey] = false`. Authorized to [`AGENT_CIRCUIT_BREAKER_ADDRESS`] only.
    pub fn untrip_key(&mut self, msg_sender: Address, call: untripKeyCall) -> Result<()> {
        if msg_sender != AGENT_CIRCUIT_BREAKER_ADDRESS {
            return Err(SciAgentStateError::unauthorized_caller().into());
        }
        self.tripped[call.sessionKey].write(false)?;
        self.emit_event(SciAgentStateEvent::TripStateUpdate(ISciAgentState::TripStateUpdate {
            sessionKey: call.sessionKey,
            isTripped: false,
        }))
    }

    /// Returns the current trip flag for `sessionKey`.
    pub fn is_tripped(&self, call: isTrippedCall) -> Result<bool> {
        self.tripped[call.sessionKey].read()
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::Address;
    use alloy_sol_types::{SolCall, SolInterface};
    use tempo_chainspec::hardfork::TempoHardfork;

    use super::*;
    use crate::{
        Precompile,
        storage::{StorageCtx, hashmap::HashMapStorageProvider},
    };

    #[test]
    fn trip_key_unauthorized_caller_reverts() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        let session_key = Address::random();
        let bad_caller = Address::random();
        assert_ne!(bad_caller, AGENT_CIRCUIT_BREAKER_ADDRESS);

        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            state.initialize()?;

            let calldata = tripKeyCall { sessionKey: session_key }.abi_encode();
            let result = state.call(&calldata, bad_caller)?;
            assert!(result.is_revert(), "expected revert for unauthorized caller");

            let decoded =
                SciAgentStateError::abi_decode(&result.bytes).expect("expected SciAgentStateError");
            assert!(matches!(decoded, SciAgentStateError::Unauthorized(_)));
            Ok(())
        })
    }

    #[test]
    fn trip_key_authorized_succeeds_and_emits_event() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        let session_key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            state.initialize()?;

            let calldata = tripKeyCall { sessionKey: session_key }.abi_encode();
            let result = state.call(&calldata, AGENT_CIRCUIT_BREAKER_ADDRESS)?;
            assert!(!result.is_revert(), "expected success");

            assert!(state.tripped[session_key].read()?);

            state.assert_emitted_events(vec![SciAgentStateEvent::TripStateUpdate(
                ISciAgentState::TripStateUpdate { sessionKey: session_key, isTripped: true },
            )]);
            Ok(())
        })
    }

    #[test]
    fn untrip_key_unauthorized_caller_reverts() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        let session_key = Address::random();
        let bad_caller = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            state.initialize()?;

            // Pre-trip via authorized caller
            let trip_data = tripKeyCall { sessionKey: session_key }.abi_encode();
            let _ = state.call(&trip_data, AGENT_CIRCUIT_BREAKER_ADDRESS)?;
            assert!(state.tripped[session_key].read()?);

            // Untrip from wrong caller — should fail and leave state intact
            let untrip_data = untripKeyCall { sessionKey: session_key }.abi_encode();
            let result = state.call(&untrip_data, bad_caller)?;
            assert!(result.is_revert());
            assert!(state.tripped[session_key].read()?);
            Ok(())
        })
    }

    #[test]
    fn untrip_key_clears_flag() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        let session_key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            state.initialize()?;

            let _ = state.call(
                &tripKeyCall { sessionKey: session_key }.abi_encode(),
                AGENT_CIRCUIT_BREAKER_ADDRESS,
            )?;
            assert!(state.tripped[session_key].read()?);

            let result = state.call(
                &untripKeyCall { sessionKey: session_key }.abi_encode(),
                AGENT_CIRCUIT_BREAKER_ADDRESS,
            )?;
            assert!(!result.is_revert());
            assert!(!state.tripped[session_key].read()?);
            Ok(())
        })
    }

    #[test]
    fn is_tripped_view_returns_current_state() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        let session_key = Address::random();
        let random_caller = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            state.initialize()?;

            // Before trip
            let calldata = isTrippedCall { sessionKey: session_key }.abi_encode();
            let result = state.call(&calldata, random_caller)?;
            assert!(!result.is_revert());
            let decoded = isTrippedCall::abi_decode_returns(&result.bytes)?;
            assert!(!decoded);

            // After trip
            let _ = state.call(
                &tripKeyCall { sessionKey: session_key }.abi_encode(),
                AGENT_CIRCUIT_BREAKER_ADDRESS,
            )?;
            let result = state.call(&calldata, random_caller)?;
            assert!(!result.is_revert());
            let decoded = isTrippedCall::abi_decode_returns(&result.bytes)?;
            assert!(decoded);
            Ok(())
        })
    }

    #[test]
    fn untrip_when_not_tripped_is_idempotent() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        let session_key = Address::random();

        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            state.initialize()?;

            let result = state.call(
                &untripKeyCall { sessionKey: session_key }.abi_encode(),
                AGENT_CIRCUIT_BREAKER_ADDRESS,
            )?;
            assert!(!result.is_revert());
            assert!(!state.tripped[session_key].read()?);
            Ok(())
        })
    }
}
