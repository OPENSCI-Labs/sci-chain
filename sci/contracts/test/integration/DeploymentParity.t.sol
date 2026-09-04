// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { AgentAccessKeyRegistry } from "../../src/agent/AgentAccessKeyRegistry.sol";
import { AgentBudgetController } from "../../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";

/// @title  DeploymentParity.t.sol
/// @notice Verifies that the runtime bytecode currently deployed at the 3 SCI
///         predeploy addresses on the live devnet **byte-equals** what local `forge
///         build` produces from the same .sol sources. Catches:
///           * source/devnet drift (someone changed a .sol without rebaking
///             genesis)
///           * accidental `--profile` mismatches (optimizer settings)
///           * stale `sci-predeploy-allocs.json` in the devnet repo
///
///         This is the **only** integration test that does NOT install precompile
///         mocks at setUp — `vm.etch` would clobber the live bytecode we want
///         to compare against. We override `setUp()` to skip the etch.
///
/// **Compiler-setting alignment**: this test is meaningful only when the local
/// `forge build` profile matches what was used to bake the predeploys into
/// genesis (`sci/devnet/export-predeploy-allocs.sh`). Default foundry.toml has
/// `via_ir = true`; if the on-chain predeploys were baked with the old
/// `via_ir = false` setting (any devnet snapshot before this commit), the
/// bytecode will not byte-equal. The test is therefore **opt-in** — set
/// `CHECK_BYTECODE_PARITY=1` to enable. After the next devnet redeploy with
/// the updated foundry.toml, this flag can be removed.
contract DeploymentParityTest is Test {
    uint256 internal constant SCI_CHAIN_ID = 42_001;

    address internal constant REGISTRY = 0xbbBbbbBB00000000000000000000000000000001;
    address internal constant BUDGET = 0xbBbBbBbB00000000000000000000000000000002;
    address internal constant BREAKER = 0xBbBbbBbB00000000000000000000000000000003;

    function setUp() public {
        if (block.chainid != SCI_CHAIN_ID) vm.skip(true);
        if (vm.envOr("CHECK_BYTECODE_PARITY", uint256(0)) == 0) vm.skip(true);
    }

    function test_AgentAccessKeyRegistry_MatchesSource() public view {
        bytes memory want = type(AgentAccessKeyRegistry).runtimeCode;
        bytes memory have = REGISTRY.code;
        assertEq(keccak256(have), keccak256(want), "Registry runtime drift");
    }

    function test_AgentBudgetController_MatchesSource() public view {
        bytes memory want = type(AgentBudgetController).runtimeCode;
        bytes memory have = BUDGET.code;
        assertEq(keccak256(have), keccak256(want), "Budget runtime drift");
    }

    function test_AgentCircuitBreaker_MatchesSource() public view {
        // The breaker bakes constructor argument (initialOwner) into storage,
        // not into the runtime bytecode — runtime code is constructor-arg-free.
        // So a pure code parity check is valid.
        bytes memory want = type(AgentCircuitBreaker).runtimeCode;
        bytes memory have = BREAKER.code;
        assertEq(keccak256(have), keccak256(want), "Breaker runtime drift");
    }
}
