//! ABI dispatch for the [`SciAgentState`] precompile.

use alloy_primitives::Address;
use alloy_sol_types::SolInterface;
use revm::precompile::PrecompileResult;
use tempo_chainspec::hardfork::TempoHardfork;
use tempo_contracts::precompiles::ISciAgentState::ISciAgentStateCalls;

use super::SciAgentState;
use crate::{Precompile, SelectorSchedule, charge_input_cost, dispatch_call, mutate_void, view};

impl Precompile for SciAgentState {
    fn call(&mut self, calldata: &[u8], msg_sender: Address) -> PrecompileResult {
        if let Some(err) = charge_input_cost(&mut self.storage, calldata) {
            return err;
        }

        dispatch_call(
            calldata,
            // All `ISciAgentState` selectors live at SCI launch (T3); no per-fork
            // additions or removals so the schedule is empty.
            &[SelectorSchedule::new(TempoHardfork::T3)],
            ISciAgentStateCalls::abi_decode,
            |call| match call {
                ISciAgentStateCalls::tripKey(call) => {
                    mutate_void(call, msg_sender, |sender, c| self.trip_key(sender, c))
                }
                ISciAgentStateCalls::untripKey(call) => {
                    mutate_void(call, msg_sender, |sender, c| self.untrip_key(sender, c))
                }
                ISciAgentStateCalls::isTripped(call) => view(call, |c| self.is_tripped(c)),
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use tempo_chainspec::hardfork::TempoHardfork;

    use super::*;
    use crate::{
        storage::{StorageCtx, hashmap::HashMapStorageProvider},
        test_util::{assert_full_coverage, check_selector_coverage},
    };

    #[test]
    fn sci_agent_state_selector_coverage() -> eyre::Result<()> {
        let mut storage = HashMapStorageProvider::new_with_spec(1, TempoHardfork::T3);
        StorageCtx::enter(&mut storage, || {
            let mut state = SciAgentState::new();
            let selectors: Vec<_> = ISciAgentStateCalls::SELECTORS.to_vec();

            let unsupported = check_selector_coverage(
                &mut state,
                &selectors,
                "ISciAgentState",
                ISciAgentStateCalls::name_by_selector,
            );

            assert_full_coverage([unsupported]);
            Ok(())
        })
    }
}
