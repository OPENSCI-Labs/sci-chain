// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { DevnetBase } from "./base/DevnetBase.sol";

import { ISCIAgentDelegator } from "../../src/interfaces/ISCIAgentDelegator.sol";
import { SCIAgentDelegator } from "../../src/integration/SCIAgentDelegator.sol";
import { MockAccountKeychain } from "../mocks/MockAccountKeychain.sol";

/// @title  DelegatorIntegrationTest
/// @notice The delegator's fail-closed property — when called WITHOUT the SCI
///         Rust pre-execution hook setting `transaction_key`, execute() must
///         revert with MissingTransactionKey. This is the second line of
///         defense behind the hook itself.
///
///         For positive-path delegator tests (where the hook DOES set the
///         transient slot), see `script/integration/AgentTxLoop.s.sol` — that
///         flow can only be exercised against the real Rust hook on a live
///         devnet, not in fork mode.
contract DelegatorIntegrationTest is DevnetBase {
    function test_Execute_RevertsWhenTransactionKeyZero() public {
        // The Mock returns 0 by default.
        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](1);
        calls[0] = ISCIAgentDelegator.Call({ target: BUDGET, value: 0, data: "" });

        vm.expectRevert(ISCIAgentDelegator.MissingTransactionKey.selector);
        SCIAgentDelegator(payable(DELEGATOR)).execute(calls);
    }

    function test_Execute_ProceedsOnceTransactionKeySet() public {
        // Simulate what the Rust hook would do: seed the keychain's transient
        // slot with a non-zero session key before execute() runs.
        MockAccountKeychain(KEYCHAIN).setTransactionKey(BOB);

        // No-op inner call to the delegator's own address (will fall through
        // execute's loop without side effects since data is empty).
        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](0);
        SCIAgentDelegator(payable(DELEGATOR)).execute(calls);

        // No assertion needed — reaching here without revert is the test.
    }

    function test_Execute_BubblesInnerRevert() public {
        MockAccountKeychain(KEYCHAIN).setTransactionKey(BOB);

        // Call a non-existent function selector on the registry — will revert.
        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](1);
        calls[0] = ISCIAgentDelegator.Call({
            target: REGISTRY,
            value: 0,
            data: hex"deadbeef"
        });

        vm.expectRevert();
        SCIAgentDelegator(payable(DELEGATOR)).execute(calls);
    }

    function test_Execute_ZeroBatch_IsNoOp() public {
        MockAccountKeychain(KEYCHAIN).setTransactionKey(BOB);
        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](0);
        SCIAgentDelegator(payable(DELEGATOR)).execute(calls);
        // No revert → pass. AgentBatchExecuted should be emitted with count=0.
    }

    // -------- Fuzz tests --------

    function testFuzz_Execute_RevertsForAnyZeroSessionKey(uint256 batchSize) public {
        batchSize = bound(batchSize, 0, 10);
        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](batchSize);
        for (uint256 i; i < batchSize; ++i) {
            calls[i] = ISCIAgentDelegator.Call({ target: BUDGET, value: 0, data: "" });
        }

        // Transaction key remains zero in the mock.
        vm.expectRevert(ISCIAgentDelegator.MissingTransactionKey.selector);
        SCIAgentDelegator(payable(DELEGATOR)).execute(calls);
    }
}
