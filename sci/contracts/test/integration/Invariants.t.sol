// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { DevnetBase } from "./base/DevnetBase.sol";

import { AgentBudgetController } from "../../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";
import { ISciAgentState } from "../../src/interfaces/ISciAgentState.sol";

/// @notice Handler that drives random state changes against the deployed predeploys.
///         The handler is registered as a breaker guardian in `setUp`, so its
///         trip/untrip calls actually mutate state (msg.sender == handler == guardian)
///         and the ghost model below tracks the expected outcome of every successful
///         call. The invariants then compare ghost vs on-chain state — a silent revert
///         or storage drift shows up as a ghost mismatch instead of being swallowed.
contract InvariantHandler {
    AgentCircuitBreaker internal breaker;
    AgentBudgetController internal budget;

    // Bounded address universe for the fuzzer (keeps state space tractable).
    address[10] public sessionKeys;

    /// Ghost model: expected trip state per session key (what the last successful
    /// trip/untrip through this handler should have produced).
    mapping(address => bool) public ghostTripped;

    /// Ghost model: expected `address(0)`-token threshold per session key, configured
    /// under this handler's account (`msg.sender == handler` for setThreshold).
    mapping(address => uint256) public ghostThreshold;

    constructor(address _breaker, address _budget) {
        breaker = AgentCircuitBreaker(_breaker);
        budget = AgentBudgetController(_budget);

        for (uint256 i; i < sessionKeys.length; ++i) {
            sessionKeys[i] = address(uint160(0x2000 + i));
        }
    }

    function trip(uint8 idx) external {
        address sk = sessionKeys[idx % sessionKeys.length];
        // Handler is a guardian — this must succeed; a revert fails the invariant run.
        breaker.trip(sk, bytes32("invariant"));
        ghostTripped[sk] = true;
    }

    function untrip(uint8 idx) external {
        address sk = sessionKeys[idx % sessionKeys.length];
        breaker.untrip(sk);
        ghostTripped[sk] = false;
    }

    function setThreshold(uint8 keyIdx, uint96 threshold) external {
        address sk = sessionKeys[keyIdx % sessionKeys.length];
        budget.setThreshold(sk, address(0), threshold);
        ghostThreshold[sk] = threshold;
    }
}

/// @title  Invariants over the P0-2 predeploy bundle
contract InvariantsTest is DevnetBase {
    InvariantHandler internal handler;

    /// An address that never configures anything — used to assert that the budget
    /// controller's per-account keying does not alias another account's writes.
    address internal constant BYSTANDER = address(0xB57A2DE2);

    function setUp() public override {
        super.setUp();
        if (block.chainid != SCI_CHAIN_ID) return;

        handler = new InvariantHandler(BREAKER, BUDGET);

        // Make the handler a real guardian so its trip/untrip calls mutate state.
        vm.prank(ALICE);
        AgentCircuitBreaker(BREAKER).setGuardian(address(handler), true);

        targetContract(address(handler));

        // Narrow the surface forge's invariant fuzzer targets.
        bytes4[] memory selectors = new bytes4[](3);
        selectors[0] = InvariantHandler.trip.selector;
        selectors[1] = InvariantHandler.untrip.selector;
        selectors[2] = InvariantHandler.setThreshold.selector;
        targetSelector(FuzzSelector({ addr: address(handler), selectors: selectors }));
    }

    /// For every session key, the facade view, the precompile view, and the ghost model
    /// must agree. The facade simply forwards to the precompile, so a facade/precompile
    /// mismatch catches storage drift between them, and a ghost mismatch catches calls
    /// that silently failed (or mutated the wrong key).
    function invariant_TripStateMirrored() public view {
        for (uint8 i; i < 10; ++i) {
            address sk = handler.sessionKeys(i);
            bool facadeView = AgentCircuitBreaker(BREAKER).isTripped(sk);
            assertEq(
                facadeView,
                ISciAgentState(SCI_AGENT_STATE).isTripped(sk),
                "facade/precompile trip-state mirror violated"
            );
            assertEq(facadeView, handler.ghostTripped(sk), "trip state diverged from ghost model");
        }
    }

    /// The breaker's owner must never become address(0) — `renounceOwnership` is
    /// disabled (it would freeze guardian management forever) and OZ Ownable forbids
    /// transferOwnership to zero.
    function invariant_OwnerNotZero() public view {
        assertTrue(AgentCircuitBreaker(BREAKER).owner() != address(0));
    }

    /// AgentBudgetController storage is keyed by (msg.sender, keyId, token): the
    /// handler's own writes must read back exactly (ghost model), and an account that
    /// never wrote anything must always read zero for the same (keyId, token) — real
    /// cross-actor aliasing would violate one of the two.
    function invariant_ThresholdNotAliased() public view {
        for (uint8 i; i < 10; ++i) {
            address sk = handler.sessionKeys(i);
            assertEq(
                AgentBudgetController(BUDGET).getThreshold(address(handler), sk, address(0)),
                handler.ghostThreshold(sk),
                "handler threshold diverged from ghost model"
            );
            assertEq(
                AgentBudgetController(BUDGET).getThreshold(BYSTANDER, sk, address(0)),
                0,
                "bystander must never observe another account's threshold"
            );
        }
    }
}
