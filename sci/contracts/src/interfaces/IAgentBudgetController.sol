// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

/// @title  IAgentBudgetController
/// @notice Predeploy at 0xBBBB..0002. Read-side facade over the keychain's
///         `getRemainingLimitWithPeriod`, plus per-(account, key, token) alert
///         thresholds. Crossing a threshold downward emits `BudgetAlert`.
///         The keychain is still the source of truth for spending limits — this
///         contract does not mutate quota state.
///
/// @dev    Plan A (native AA tx type, D-gas / D2-B) semantics: the `address(0)`
///         token is a sentinel quota that meters BOTH a transaction's native
///         `value` transfers AND its gas spend (`gas_used * max_fee`, deducted by
///         the pre-execution hook against the root account). So
///         `remaining(account, keyId, address(0))` is the agent's combined
///         native+gas budget, not merely a native-transfer budget — query it via
///         the [`gasBudget`] convenience accessor. ERC-20 (incl. SCI-20
///         `transferWithMemo`) limits remain per-token under the token's own
///         address (D3-B).
interface IAgentBudgetController {
    event ThresholdConfigured(
        address indexed account, address indexed keyId, address indexed token, uint256 threshold
    );
    event BudgetAlert(
        address indexed account,
        address indexed keyId,
        address indexed token,
        uint256 remaining,
        uint256 threshold
    );

    error UnauthorizedCaller();

    function setThreshold(address keyId, address token, uint256 threshold) external;

    function getThreshold(address account, address keyId, address token)
        external
        view
        returns (uint256);

    function remaining(address account, address keyId, address token)
        external
        view
        returns (uint256 amount, uint64 periodEnd);

    /// Convenience accessor for the combined native+gas (`address(0)` sentinel)
    /// budget under Plan A D-gas. Equivalent to `remaining(account, keyId,
    /// address(0))`. Returns the remaining quota and the limit's period end.
    function gasBudget(address account, address keyId)
        external
        view
        returns (uint256 amount, uint64 periodEnd);

    /// Reads current remaining quota and emits `BudgetAlert` if it is at or below the
    /// configured threshold. Callable by anyone; safe by virtue of being a pure
    /// observation event. Returns the values it read for caller convenience.
    function checkAndAlert(address account, address keyId, address token)
        external
        returns (uint256 remainingAmount, uint64 periodEnd, bool alerted);
}
