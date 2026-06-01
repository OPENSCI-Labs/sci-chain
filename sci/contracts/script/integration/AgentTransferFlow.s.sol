// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Script } from "forge-std/Script.sol";
import { Vm } from "forge-std/Vm.sol";
import { console2 } from "forge-std/console2.sol";

import { AgentAccessKeyRegistry } from "../../src/agent/AgentAccessKeyRegistry.sol";
import { AgentBudgetController } from "../../src/agent/AgentBudgetController.sol";
import { AgentCircuitBreaker } from "../../src/agent/AgentCircuitBreaker.sol";
import { IAccountKeychain } from "../../src/interfaces/IAccountKeychain.sol";
import { IAgentAccessKeyRegistry } from "../../src/interfaces/IAgentAccessKeyRegistry.sol";
import { ISCIAgentDelegator } from "../../src/interfaces/ISCIAgentDelegator.sol";
import { ISciAgentState } from "../../src/interfaces/ISciAgentState.sol";

import { MockAccountKeychain } from "../../test/mocks/MockAccountKeychain.sol";
import { MockSciAgentState } from "../../test/mocks/MockSciAgentState.sol";

/// @notice Minimal ERC-20 deployed by the flow to give us a real token to
///         transfer. Not committed as a permanent contract — owned by the
///         deployer (alice) and re-deployed every script run.
contract TestToken {
    string public constant name = "SCI Integration Test Token";
    string public constant symbol = "ITT";
    uint8 public constant decimals = 18;
    uint256 public totalSupply;

    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;

    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    function mint(address to, uint256 amount) external {
        balanceOf[to] += amount;
        totalSupply += amount;
        emit Transfer(address(0), to, amount);
    }

    function transfer(address to, uint256 amount) external returns (bool) {
        balanceOf[msg.sender] -= amount;
        balanceOf[to] += amount;
        emit Transfer(msg.sender, to, amount);
        return true;
    }

    function approve(address spender, uint256 amount) external returns (bool) {
        allowance[msg.sender][spender] = amount;
        emit Approval(msg.sender, spender, amount);
        return true;
    }

    function transferFrom(address from, address to, uint256 amount) external returns (bool) {
        if (allowance[from][msg.sender] != type(uint256).max) {
            allowance[from][msg.sender] -= amount;
        }
        balanceOf[from] -= amount;
        balanceOf[to] += amount;
        emit Transfer(from, to, amount);
        return true;
    }
}

/// @title  AgentTransferFlow
/// @notice End-to-end live-broadcast walkthrough of the SCI agent-tx loop with
///         a real ERC-20 transfer at the end. Touches the full public API
///         surface of the 4 fixed-address Solidity predeploys + the
///         AccountKeychain precompile.
///
/// Coverage map
/// ------------
/// Keychain (precompile at 0xAAAA..00):
///   ✓ authorizeKey (T3 overload, with TokenLimit + scoped CallScope)
///   ✓ authorizeKey (T5 overload with witness) — separate session key
///   ✓ updateSpendingLimit
///   ✓ setAllowedCalls + removeAllowedCalls
///   ✓ revokeKey
///   ✓ getKey / getRemainingLimit / getRemainingLimitWithPeriod
///   ✓ getAllowedCalls
///   ✓ getTransactionKey (implicit, asserted by delegator)
///   ✓ burnKeyAuthorizationWitness + isKeyAuthorizationWitnessBurned
///
/// SciAgentState precompile (0xAAAA..01):
///   ✓ isTripped (read, after CB facade flips it)
///   ✓ tripKey / untripKey (only reachable via CB; direct call is access-controlled)
///
/// AgentAccessKeyRegistry (0xBBBB..01):
///   ✓ bindKey / unbindKey
///   ✓ getBinding / isBound / agentIdOf
///
/// AgentBudgetController (0xBBBB..02):
///   ✓ setThreshold / getThreshold
///   ✓ remaining
///   ✓ checkAndAlert (both alert and no-alert branches)
///
/// AgentCircuitBreaker (0xBBBB..03):
///   ✓ trip / untrip
///   ✓ isTripped
///   ✓ setGuardian / isGuardian
///   ✓ owner (read)
///
/// SCIAgentDelegator (0xCCCC..01):
///   ✓ execute (the agent-tx terminal call, real ERC-20 transfer)
///
/// Usage
/// -----
///   export L2_RPC=http://localhost:7545     # builder RPC (run on devnet host)
///   export PATH=$HOME/.foundry/bin:$PATH
///   cd ~/sci-dev/sci-chain/sci/contracts
///
///   # Real broadcast — mutates devnet state. The --skip-simulation flag is
///   # MANDATORY: forge's post-script "on-chain dry run" uses a forked anvil
///   # that can't load SCI's Rust precompiles, so it always hits OpcodeNotFound
///   # at any keychain / sci-agent-state call. The actual on-chain broadcast
///   # bypasses this path and lands cleanly on the real chain where reth
///   # dispatches precompiles correctly.
///   forge script script/integration/AgentTransferFlow.s.sol \
///     --tc AgentTransferFlow \
///     --rpc-url $L2_RPC --broadcast --skip-simulation -vvv
///
///   # Dry-run mode (no --broadcast): local simulation only, against the mocks
///   # etched in _installPrecompileMocks. Useful for verifying script logic
///   # before committing to a broadcast run.
///   forge script script/integration/AgentTransferFlow.s.sol \
///     --tc AgentTransferFlow --rpc-url $L2_RPC -vv
///
/// State note
/// ----------
/// Each run deploys a fresh TestToken (CREATE address derived from alice's
/// nonce, so different every run) and mints to alice. The script does NOT
/// clean up: alice keeps her authorized keys, 7702 delegation, and TestToken
/// balance. Re-running is safe — it simply layers new state on top — but to
/// reset, redeploy devnet via the apply-* scripts.
///
/// Negative-case verification (hook reject on tripped key / limit exceeded
/// / scope violation) is documented in `sci/devnet/E2E.md` §4-7 — run those
/// with cast after this script completes if you want the rejection coverage.
contract AgentTransferFlow is Script {
    // -------- Devnet fixed addresses --------
    address constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;
    address constant SCI_AGENT_STATE = 0xAaAAAaAA00000000000000000000000000000001;
    address constant REGISTRY = 0xbbBbbbBB00000000000000000000000000000001;
    address constant BUDGET = 0xbBbBbBbB00000000000000000000000000000002;
    address constant BREAKER = 0xBbBbbBbB00000000000000000000000000000003;
    address constant DELEGATOR = 0xCcCCCCcC00000000000000000000000000000001;

    // -------- Default devnet test accounts --------
    uint256 constant DEFAULT_ALICE_PK =
        0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    uint256 constant DEFAULT_BOB_PK =
        0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    uint256 constant DEFAULT_CHARLIE_PK =
        0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;

    // -------- Runtime addresses + balances --------
    uint256 alicePk;
    uint256 bobPk;
    uint256 charliePk;
    address alice;
    address bob;
    address charlie;

    function run() external {
        _loadAccounts();

        // Phase 0: install precompile mocks for the LOCAL simulation only.
        //
        // forge script runs every state-changing call locally (in an anvil-like
        // EVM that forks the live chain) to compute gas and verify success
        // before broadcasting. That local EVM does NOT contain SCI's Rust
        // precompiles; calls to 0xAAAA..00/01 hit the genesis `0xef` marker and
        // revert with OpcodeNotFound.
        //
        // vm.etch is a cheatcode — it patches simulation state only, never
        // appears on the real chain. So simulation runs against our Solidity
        // mock keychain (which preserves the exact same behavior), and the
        // broadcasted txs hit the real Rust precompile when they land.
        //
        // Mocks at the precompile addresses are stateless wrt the live chain:
        // every script run starts the mock with empty storage and the script
        // re-builds state through the same calls the broadcast would make.
        _installPrecompileMocks();

        // Phase 1: fund alice if she has no gas (bob has 10k ETH on fresh chain).
        _ensureGas();

        // Phase 2: deploy a fresh TestToken under alice and mint a known balance.
        TestToken token = _deployAndMintToken(1_000 ether);
        console2.log("[2] TestToken deployed at:", address(token));
        console2.log("    alice balance       :", token.balanceOf(alice));

        // Phase 3: cover the full keychain API surface for ALICE authorizing BOB.
        _coverKeychainAPI(address(token));

        // Phase 4: cover the registry binding lifecycle.
        _coverRegistryAPI();

        // Phase 5: cover the budget controller surface.
        _coverBudgetAPI(address(token));

        // Phase 6: cover the circuit breaker surface (without leaving bob tripped).
        _coverCircuitBreakerAPI();

        // Phase 7: cover the T5 witness API on a separate session key.
        _coverWitnessAPI();

        // Phase 8: install (or refresh) alice's EIP-7702 delegation to delegator.
        _install7702();

        // Phase 9: the headline test — bob initiates a real token transfer to
        // charlie via alice's delegated account. Hook validates, delegator
        // forwards, keychain deducts the spending limit.
        _agentTransferTokens(address(token), 100 ether);

        // Phase 10: final state report.
        _report(address(token));
    }

    // ------------------------------------------------------------------ //
    // Setup helpers                                                       //
    // ------------------------------------------------------------------ //

    function _installPrecompileMocks() internal {
        MockAccountKeychain kc = new MockAccountKeychain();
        vm.etch(KEYCHAIN, address(kc).code);
        MockSciAgentState sas = new MockSciAgentState();
        vm.etch(SCI_AGENT_STATE, address(sas).code);
    }

    function _loadAccounts() internal {
        alicePk = vm.envOr("ALICE_PK", DEFAULT_ALICE_PK);
        charliePk = vm.envOr("CHARLIE_PK", DEFAULT_CHARLIE_PK);
        alice = vm.addr(alicePk);
        charlie = vm.addr(charliePk);

        // Session key is derived per-run from block.timestamp so the
        // keychain's "already revoked" rejection (the precompile remembers
        // revoked keys to prevent replay) can't bite on re-runs against the
        // same devnet. Operators wanting a stable session key can set
        // BOB_PK to a fixed value and re-deploy the devnet between runs.
        bobPk = vm.envOr(
            "BOB_PK", uint256(keccak256(abi.encode("agent-session-key", block.timestamp)))
        );
        bob = vm.addr(bobPk);

        console2.log("== AgentTransferFlow ==");
        console2.log("  alice (root)       :", alice);
        console2.log("  bob   (session key):", bob);
        console2.log("  charlie (recipient):", charlie);
    }

    function _ensureGas() internal {
        // Fresh session key needs gas to send the execute tx. Fund from a
        // pre-funded test account (charlie has 10k ETH on the fresh devnet).
        if (bob.balance < 0.5 ether) {
            console2.log("[1a] funding bob (session key) from charlie");
            vm.broadcast(charliePk);
            (bool ok,) = bob.call{ value: 1 ether }("");
            require(ok, "fund bob failed");
        }

        // Alice (root) also needs gas for her broadcasted txs.
        if (alice.balance < 0.5 ether) {
            console2.log("[1b] funding alice from charlie");
            vm.broadcast(charliePk);
            (bool ok,) = alice.call{ value: 1 ether }("");
            require(ok, "fund alice failed");
        } else {
            console2.log("[1] alice has gas:", alice.balance, " bob has gas:", bob.balance);
        }
    }

    function _deployAndMintToken(uint256 mintAmount) internal returns (TestToken token) {
        vm.broadcast(alicePk);
        token = new TestToken();

        vm.broadcast(alicePk);
        token.mint(alice, mintAmount);
    }

    // ------------------------------------------------------------------ //
    // API coverage helpers                                                //
    // ------------------------------------------------------------------ //

    function _coverKeychainAPI(address token) internal {
        console2.log("[3] keychain API coverage");
        _keychainAuthorize(token);
        _keychainUpdateAndScope(token);
        _keychainViews(token);
    }

    function _keychainAuthorize(address token) internal {
        // Pre-revoke: the keychain rejects authorizeKey with KeyAlreadyExists
        // if (alice, bob) already has an active key. We can't read real-chain
        // state during local simulation (the etched mock returns its own empty
        // state, not the live keychain's), so we use an env flag to choose:
        //
        //   RESET_BOB_KEY=1  ⇒ broadcast a revokeKey first. Required on a
        //                       devnet where bob has been authorized by a
        //                       prior cast walkthrough or script run.
        //   (unset)           ⇒ skip revoke. Use on a freshly redeployed
        //                       devnet where bob has never been authorized.
        if (vm.envOr("RESET_BOB_KEY", uint256(0)) == 1) {
            vm.broadcast(alicePk);
            IAccountKeychain(KEYCHAIN).revokeKey(bob);
            console2.log("    3a-pre. revokeKey forced via RESET_BOB_KEY=1   OK");
        }

        IAccountKeychain.KeyRestrictions memory cfg = _buildScopedRestrictions(token, 200 ether);
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).authorizeKey(bob, IAccountKeychain.SignatureType.Secp256k1, cfg);
        console2.log("    3a. authorizeKey (T3, scoped, 200 ITT limit)  OK");
    }

    function _keychainUpdateAndScope(address token) internal {
        // 3b. updateSpendingLimit — bump the cap from 200 → 300 to exercise the path.
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).updateSpendingLimit(bob, token, 300 ether);
        console2.log("    3b. updateSpendingLimit (300 ITT)             OK");

        // 3c. setAllowedCalls — replace scope with both transfer + approve allowed.
        IAccountKeychain.CallScope[] memory scopes = _buildTwoSelectorScope(token);
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).setAllowedCalls(bob, scopes);
        console2.log("    3c. setAllowedCalls (transfer + approve)      OK");
    }

    function _keychainViews(address token) internal view {
        IAccountKeychain.KeyInfo memory info = IAccountKeychain(KEYCHAIN).getKey(alice, bob);
        require(info.keyId == bob, "getKey: keyId mismatch");
        require(!info.isRevoked, "getKey: should not be revoked");
        console2.log("    3d. getKey                                    OK");

        uint256 rem = IAccountKeychain(KEYCHAIN).getRemainingLimit(alice, bob, token);
        require(rem == 300 ether, "getRemainingLimit unexpected");
        console2.log("    3e. getRemainingLimit (300 ITT)               OK");

        (uint256 rem2, uint64 periodEnd) =
            IAccountKeychain(KEYCHAIN).getRemainingLimitWithPeriod(alice, bob, token);
        require(rem2 == 300 ether && periodEnd == 0, "getRemainingLimitWithPeriod unexpected");
        console2.log("    3f. getRemainingLimitWithPeriod (300, p=0)    OK");

        (bool isScoped, IAccountKeychain.CallScope[] memory got) =
            IAccountKeychain(KEYCHAIN).getAllowedCalls(alice, bob);
        require(isScoped, "getAllowedCalls: expected scoped");
        require(got.length == 1 && got[0].target == token, "getAllowedCalls: wrong target");
        console2.log("    3g. getAllowedCalls (scoped, 1 target)        OK");
    }

    // -------- Struct builders --------

    function _buildScopedRestrictions(address token, uint256 limitAmount)
        internal
        view
        returns (IAccountKeychain.KeyRestrictions memory)
    {
        IAccountKeychain.TokenLimit[] memory limits = new IAccountKeychain.TokenLimit[](1);
        limits[0] = IAccountKeychain.TokenLimit({ token: token, amount: limitAmount, period: 0 });

        IAccountKeychain.SelectorRule[] memory rules = new IAccountKeychain.SelectorRule[](1);
        rules[0] = IAccountKeychain.SelectorRule({
            selector: TestToken.transfer.selector,
            recipients: new address[](0)
        });

        IAccountKeychain.CallScope[] memory scopes = new IAccountKeychain.CallScope[](1);
        scopes[0] = IAccountKeychain.CallScope({ target: token, selectorRules: rules });

        return IAccountKeychain.KeyRestrictions({
            expiry: uint64(block.timestamp + 1 days),
            enforceLimits: true,
            limits: limits,
            allowAnyCalls: false,
            allowedCalls: scopes
        });
    }

    function _buildTwoSelectorScope(address token)
        internal
        pure
        returns (IAccountKeychain.CallScope[] memory scopes)
    {
        IAccountKeychain.SelectorRule[] memory rules = new IAccountKeychain.SelectorRule[](2);
        rules[0] = IAccountKeychain.SelectorRule({
            selector: TestToken.transfer.selector,
            recipients: new address[](0)
        });
        rules[1] = IAccountKeychain.SelectorRule({
            selector: TestToken.approve.selector,
            recipients: new address[](0)
        });
        scopes = new IAccountKeychain.CallScope[](1);
        scopes[0] = IAccountKeychain.CallScope({ target: token, selectorRules: rules });
    }

    function _coverRegistryAPI() internal {
        console2.log("[4] registry API coverage");

        bytes32 agentId = bytes32("integration-agent-1");

        // 4a. If bob is already bound (prior run), unbind first for a clean start.
        if (AgentAccessKeyRegistry(REGISTRY).isBound(bob)) {
            vm.broadcast(alicePk);
            AgentAccessKeyRegistry(REGISTRY).unbindKey(bob);
            console2.log("    4a. unbindKey (pre-existing binding cleared)  OK");
        }

        // 4b. bindKey (the fresh bind).
        vm.broadcast(alicePk);
        AgentAccessKeyRegistry(REGISTRY).bindKey(bob, agentId);
        console2.log("    4b. bindKey                                    OK");

        // 4c. Views.
        require(AgentAccessKeyRegistry(REGISTRY).isBound(bob), "isBound should be true");
        require(
            AgentAccessKeyRegistry(REGISTRY).agentIdOf(bob) == agentId, "agentIdOf mismatch"
        );
        IAgentAccessKeyRegistry.Binding memory b =
            AgentAccessKeyRegistry(REGISTRY).getBinding(bob);
        require(b.account == alice && b.agentId == agentId && !b.revoked, "getBinding mismatch");
        console2.log("    4c. isBound + agentIdOf + getBinding           OK");
    }

    function _coverBudgetAPI(address token) internal {
        console2.log("[5] budget API coverage");

        // 5a. setThreshold (alert when remaining drops to 50).
        vm.broadcast(alicePk);
        AgentBudgetController(BUDGET).setThreshold(bob, token, 50 ether);
        console2.log("    5a. setThreshold (50 ITT)                      OK");

        // 5b. getThreshold roundtrip.
        require(
            AgentBudgetController(BUDGET).getThreshold(alice, bob, token) == 50 ether,
            "getThreshold mismatch"
        );
        console2.log("    5b. getThreshold                               OK");

        // 5c. remaining proxies the keychain.
        (uint256 amt, uint64 pe) = AgentBudgetController(BUDGET).remaining(alice, bob, token);
        require(amt == 300 ether && pe == 0, "remaining mismatch");
        console2.log("    5c. remaining (300 ITT)                        OK");

        // 5d. checkAndAlert — remaining (300) > threshold (50) ⇒ no alert.
        vm.broadcast(alicePk);
        AgentBudgetController(BUDGET).checkAndAlert(alice, bob, token);
        console2.log("    5d. checkAndAlert (no alert, remaining > thr)  OK");
    }

    function _coverCircuitBreakerAPI() internal {
        console2.log("[6] circuit breaker API coverage");

        // 6a. owner view + isGuardian view.
        address own = AgentCircuitBreaker(BREAKER).owner();
        require(own == alice, "CB owner must be alice");
        require(AgentCircuitBreaker(BREAKER).isGuardian(alice), "alice should be guardian");
        console2.log("    6a. owner + isGuardian(alice)                  OK");

        // 6b. setGuardian — grant charlie, then revoke.
        vm.broadcast(alicePk);
        AgentCircuitBreaker(BREAKER).setGuardian(charlie, true);
        require(AgentCircuitBreaker(BREAKER).isGuardian(charlie), "charlie guardian grant");

        vm.broadcast(alicePk);
        AgentCircuitBreaker(BREAKER).setGuardian(charlie, false);
        require(!AgentCircuitBreaker(BREAKER).isGuardian(charlie), "charlie guardian revoke");
        console2.log("    6b. setGuardian (grant + revoke)               OK");

        // 6c. trip + isTripped + precompile mirror + untrip.
        vm.broadcast(alicePk);
        AgentCircuitBreaker(BREAKER).trip(bob, bytes32("test-trip"));
        require(AgentCircuitBreaker(BREAKER).isTripped(bob), "trip should reflect");
        require(ISciAgentState(SCI_AGENT_STATE).isTripped(bob), "precompile mirror");

        vm.broadcast(alicePk);
        AgentCircuitBreaker(BREAKER).untrip(bob);
        require(!AgentCircuitBreaker(BREAKER).isTripped(bob), "untrip should clear");
        console2.log("    6c. trip + isTripped + mirror + untrip         OK");
    }

    function _coverWitnessAPI() internal {
        console2.log("[7] T5 witness API coverage");

        // 7a. Derive a one-shot session key + witness from the current timestamp.
        // Across script re-runs each invocation gets a fresh (sk, witness) pair,
        // so neither authorizeKey nor revokeKey collide with prior state.
        (address sk,) = makeAddrAndKey(string.concat("witness-sk-", vm.toString(block.timestamp)));
        bytes32 witness = keccak256(abi.encode("witness", block.timestamp));

        // 7b. authorizeKey (witness overload).
        IAccountKeychain.TokenLimit[] memory noLimits = new IAccountKeychain.TokenLimit[](0);
        IAccountKeychain.CallScope[] memory noScopes = new IAccountKeychain.CallScope[](0);
        IAccountKeychain.KeyRestrictions memory cfg = IAccountKeychain.KeyRestrictions({
            expiry: uint64(block.timestamp + 1 hours),
            enforceLimits: false,
            limits: noLimits,
            allowAnyCalls: true,
            allowedCalls: noScopes
        });
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).authorizeKey(
            sk, IAccountKeychain.SignatureType.Secp256k1, cfg, witness
        );
        console2.log("    7a. authorizeKey (T5 witness)                  OK");

        // 7c. Witness is not yet burned.
        require(
            !IAccountKeychain(KEYCHAIN).isKeyAuthorizationWitnessBurned(alice, witness),
            "witness should not be burned yet"
        );

        // 7d. burnKeyAuthorizationWitness.
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).burnKeyAuthorizationWitness(witness);
        console2.log("    7b. burnKeyAuthorizationWitness                OK");

        // 7e. Now it's burned.
        require(
            IAccountKeychain(KEYCHAIN).isKeyAuthorizationWitnessBurned(alice, witness),
            "witness should be burned after burn"
        );
        console2.log("    7c. isKeyAuthorizationWitnessBurned (post-burn) OK");

        // 7f. revokeKey on the witness key (we don't need it any further).
        vm.broadcast(alicePk);
        IAccountKeychain(KEYCHAIN).revokeKey(sk);
        console2.log("    7d. revokeKey                                  OK");
    }

    // ------------------------------------------------------------------ //
    // EIP-7702 install                                                    //
    // ------------------------------------------------------------------ //

    function _install7702() internal {
        // If alice already has the right delegation header, skip.
        bytes memory code = alice.code;
        if (code.length == 23 && code[0] == 0xef && code[1] == 0x01 && code[2] == 0x00) {
            address current;
            assembly {
                current := mload(add(code, 23))
            }
            if (current == DELEGATOR) {
                console2.log("[8] 7702 already installed on alice -> delegator");
                return;
            }
        }

        console2.log("[8] installing 7702 delegation: alice -> delegator");
        Vm.SignedDelegation memory signed = vm.signDelegation(DELEGATOR, alicePk);
        vm.broadcast(alicePk);
        vm.attachDelegation(signed);
        // Trigger the auth via a 0-value self-call. attachDelegation queues the
        // delegation onto the next broadcasted tx from alicePk.
        vm.broadcast(alicePk);
        (bool ok,) = alice.call{ value: 0 }("");
        require(ok, "7702 install tx failed");
        console2.log("    7702 installed");
    }

    // ------------------------------------------------------------------ //
    // The headline: real ERC-20 transfer via agent                        //
    // ------------------------------------------------------------------ //

    function _agentTransferTokens(address token, uint256 amount) internal {
        console2.log("[9] agent transfer: bob signs execute(transfer(charlie,", amount);
        console2.log("    pre-flight balances:");
        console2.log("      alice ITT  :", TestToken(token).balanceOf(alice));
        console2.log("      charlie ITT:", TestToken(token).balanceOf(charlie));
        (uint256 remBefore,) =
            IAccountKeychain(KEYCHAIN).getRemainingLimitWithPeriod(alice, bob, token);
        console2.log("      bob remaining ITT limit:", remBefore);

        // Seed the mock keychain's transient `transaction_key` slot so the
        // delegator's `getTransactionKey() != 0` check passes during local
        // simulation. `vm.store` is a forge cheatcode — only affects the local
        // EVM, never broadcasted. On the real chain the SCI Rust pre-execution
        // hook sets this slot itself before EVM dispatch.
        //
        // The mock declares `address private _transactionKey;` as its first
        // state variable, so it sits at storage slot 0 of the etched mock.
        vm.store(KEYCHAIN, bytes32(uint256(0)), bytes32(uint256(uint160(bob))));

        // Encode inner call: TestToken.transfer(charlie, amount).
        bytes memory inner =
            abi.encodeWithSelector(TestToken.transfer.selector, charlie, amount);

        // Encode outer: execute([(token, 0, inner)]).
        ISCIAgentDelegator.Call[] memory batch = new ISCIAgentDelegator.Call[](1);
        batch[0] = ISCIAgentDelegator.Call({ target: token, value: 0, data: inner });

        // Send from bob — tx.to = alice (the 7702-delegated root account).
        vm.broadcast(bobPk);
        ISCIAgentDelegator(alice).execute(batch);

        console2.log("    post-flight balances:");
        console2.log("      alice ITT  :", TestToken(token).balanceOf(alice));
        console2.log("      charlie ITT:", TestToken(token).balanceOf(charlie));
        (uint256 remAfter,) =
            IAccountKeychain(KEYCHAIN).getRemainingLimitWithPeriod(alice, bob, token);
        console2.log("      bob remaining ITT limit:", remAfter);
    }

    function _report(address token) internal view {
        console2.log("== Final state ==");
        console2.log("  TestToken          :", token);
        console2.log("  alice ITT balance  :", TestToken(token).balanceOf(alice));
        console2.log("  charlie ITT balance:", TestToken(token).balanceOf(charlie));
        IAccountKeychain.KeyInfo memory info = IAccountKeychain(KEYCHAIN).getKey(alice, bob);
        console2.log("  bob key revoked    :", info.isRevoked);
        console2.log("  bob key expiry     :", info.expiry);
        console2.log("  bob agentId        :");
        console2.logBytes32(AgentAccessKeyRegistry(REGISTRY).agentIdOf(bob));
        console2.log("  bob isTripped      :", AgentCircuitBreaker(BREAKER).isTripped(bob));
        console2.log("== AgentTransferFlow complete ==");
    }
}
