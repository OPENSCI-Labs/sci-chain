// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { IAccountKeychain } from "../interfaces/IAccountKeychain.sol";
import { IAgentBudgetController } from "../interfaces/IAgentBudgetController.sol";

/// @title  AgentBudgetController
/// @notice Predeploy at 0xBBBB..0002. Read-side facade over the keychain spending
///         limits, plus an optional per-(account, keyId, token) alert threshold. The
///         keychain remains the source of truth for limit deduction — this contract
///         only observes quota state and emits events when an account drops to or
///         below its configured threshold.
///
/// @dev    `setThreshold` is authorized to `msg.sender == account`. The root account
///         owns its own monitoring config; registrars/gateways can pre-seed via the
///         same root key.
///
///         Plan A D-gas note: under the native AA tx type, the pre-execution hook
///         meters native `value` AND gas (`gas_used * max_fee`) into the keychain's
///         `address(0)` sentinel limit (see [`GAS_TOKEN`]). The `remaining` view
///         therefore reflects gas burn when queried for that token; [`gasBudget`]
///         exposes it directly.
contract AgentBudgetController is IAgentBudgetController {
    address internal constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;

    /// Sentinel token for the combined native-value + gas budget (Plan A D2-B / D-gas).
    /// The keychain meters both into the `address(0)` limit.
    address public constant GAS_TOKEN = address(0);

    /// account → keyId → token → threshold
    mapping(address => mapping(address => mapping(address => uint256))) private _thresholds;

    function setThreshold(address keyId, address token, uint256 threshold) external {
        _thresholds[msg.sender][keyId][token] = threshold;
        emit ThresholdConfigured(msg.sender, keyId, token, threshold);
    }

    function getThreshold(address account, address keyId, address token)
        external
        view
        returns (uint256)
    {
        return _thresholds[account][keyId][token];
    }

    function remaining(address account, address keyId, address token)
        external
        view
        returns (uint256 amount, uint64 periodEnd)
    {
        return IAccountKeychain(KEYCHAIN).getRemainingLimitWithPeriod(account, keyId, token);
    }

    function gasBudget(address account, address keyId)
        external
        view
        returns (uint256 amount, uint64 periodEnd)
    {
        return IAccountKeychain(KEYCHAIN).getRemainingLimitWithPeriod(account, keyId, GAS_TOKEN);
    }

    function checkAndAlert(address account, address keyId, address token)
        external
        returns (uint256 remainingAmount, uint64 periodEnd, bool alerted)
    {
        (remainingAmount, periodEnd) =
            IAccountKeychain(KEYCHAIN).getRemainingLimitWithPeriod(account, keyId, token);

        uint256 threshold = _thresholds[account][keyId][token];
        if (threshold != 0 && remainingAmount <= threshold) {
            emit BudgetAlert(account, keyId, token, remainingAmount, threshold);
            alerted = true;
        }
    }
}
