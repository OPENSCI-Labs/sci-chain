// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAccountKeychain } from "../interfaces/IAccountKeychain.sol";
import { ISCIAgentDelegator } from "../interfaces/ISCIAgentDelegator.sol";

/// @title  SCIAgentDelegator
/// @notice **Plan B (legacy) compatibility layer — NOT on the Plan A hot path.**
///         Under Plan A (native AA tx type `0x76`), the agent's batch rides the tx
///         itself as `BaseAaTransaction.calls[]` and is executed atomically by the
///         Rust handler (`SciHandler::execute_aa_batch`) with the keychain checks
///         applied pre-execution — there is no EIP-7702 delegation and no call into
///         this contract. This predeploy is retained only for the Plan B path
///         (standard EIP-1559 tx + EIP-7702 delegation to this address) so existing
///         tooling/tests that pre-date the AA tx type still function. New agent
///         flows should use the AA tx type and bypass it entirely.
///
/// @notice EIP-7702 batch executor predeploy at 0xCCCC..0001. An agent root account
///         delegates here via EIP-7702; the agent's session key signs a tx to the root
///         account with `tx.input = execute(Call[])`. Two gates protect the call:
///
///         1) The Rust pre-execution hook (`SciHandler`) recognizes the tx as an agent
///            tx (7702 delegate matches this address + a non-revoked keychain key for
///            (root, session_key) exists), validates each `Call`'s scope, pre-flights
///            spending limits, and sets the keychain's transient `transaction_key`
///            slot to `session_key`.
///         2) `execute()` reads `transaction_key` and reverts if zero — so any call to
///            `execute()` that bypassed the hook (direct call, non-7702 path, etc.)
///            fails closed before any inner call runs.
///
/// @dev    With 7702, `address(this)` inside `execute()` is the agent root account, so
///         downstream `target.call(...)` carries the root account as `msg.sender`. The
///         delegator forwards `value` and bubbles raw revert data via `CallReverted`.
///         Atomic-batch semantics: the first inner failure reverts the whole batch.
contract SCIAgentDelegator is ISCIAgentDelegator {
    /// Address of the AccountKeychain precompile.
    address internal constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    function execute(Call[] calldata calls) external payable {
        address sessionKey = IAccountKeychain(KEYCHAIN).getTransactionKey();
        if (sessionKey == address(0)) revert MissingTransactionKey();

        uint256 len = calls.length;
        for (uint256 i = 0; i < len;) {
            Call calldata c = calls[i];
            (bool ok, bytes memory ret) = c.target.call{ value: c.value }(c.data);
            if (!ok) revert CallReverted(i, ret);
            emit AgentCallExecuted(address(this), i, c.target, c.value);
            unchecked {
                ++i;
            }
        }

        emit AgentBatchExecuted(address(this), sessionKey, len);
    }

    receive() external payable { }
}
