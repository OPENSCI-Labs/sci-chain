// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { IAccountKeychain } from "../../../src/interfaces/IAccountKeychain.sol";
import { ISciAgentState } from "../../../src/interfaces/ISciAgentState.sol";
import { MockAccountKeychain } from "../../mocks/MockAccountKeychain.sol";
import { MockSciAgentState } from "../../mocks/MockSciAgentState.sol";

/// @notice Shared base for tests that target the **live SCI devnet via forge --fork-url**.
///
/// Strategy
/// --------
/// * Forking pulls genesis state, including the runtime bytecode at the 4 SCI
///   predeploy addresses (`0xBBBB..01/02/03`, `0xCCCC..01`). Tests therefore exercise
///   the **real deployed bytecode** at those addresses, not a fresh local deploy.
/// * The two precompile addresses (`0xAAAA..00/01`) hold a 1-byte `0xef` marker on
///   the live chain — the actual implementation is Rust code in the EL that a
///   forked anvil cannot reproduce. We therefore `vm.etch` mock implementations
///   over those addresses at setUp so cross-contract calls keep working.
/// * Tests that need the real Rust pre-execution hook (CircuitBreaker check inside
///   the hook, scope/spending-limit enforcement) live in `script/integration/`
///   instead and run via `forge script --broadcast` against the real chain.
///
/// To run:
///     forge test --fork-url $L2_RPC --match-path 'test/integration/**'
///
/// Tests skip themselves automatically (via setUp's chainid check) when run on
/// the default local EVM, so `forge test` without `--fork-url` won't fail.
abstract contract DevnetBase is Test {
    // -------- Devnet identity --------
    uint256 internal constant SCI_CHAIN_ID = 42_001;

    // -------- SCI fixed addresses (lowercase form; Solidity literals require EIP-55) --------
    address internal constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;
    address internal constant SCI_AGENT_STATE = 0xAaAAAaAA00000000000000000000000000000001;
    address internal constant REGISTRY = 0xbbBbbbBB00000000000000000000000000000001;
    address internal constant BUDGET = 0xbBbBbBbB00000000000000000000000000000002;
    address internal constant BREAKER = 0xBbBbbBbB00000000000000000000000000000003;
    address internal constant DELEGATOR = 0xCcCCCCcC00000000000000000000000000000001;

    // -------- Devnet test accounts (mnemonic "test test test ... junk") --------
    address internal constant ALICE = 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266;
    uint256 internal constant ALICE_PK =
        0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80;
    address internal constant BOB = 0x70997970C51812dc3A010C7d01b50e0d17dc79C8;
    uint256 internal constant BOB_PK =
        0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d;
    address internal constant CHARLIE = 0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC;
    uint256 internal constant CHARLIE_PK =
        0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a;

    // -------- Common test fixture --------
    address internal constant NATIVE_TOKEN = address(0);

    /// Skips the test body if not running against an SCI-Chain-ID fork. Concrete
    /// subclasses MAY override and call super.setUp() before their own setup.
    function setUp() public virtual {
        if (block.chainid != SCI_CHAIN_ID) {
            vm.skip(true);
            return;
        }

        // Replace the precompile markers with Solidity stand-ins so contract-to-
        // precompile calls succeed under the forked EVM (which has no SCI Rust
        // dispatch). vm.etch only touches code; storage at the precompile slots
        // starts empty in each test, isolating tests from each other and from
        // any state the live chain has accumulated.
        _installPrecompileMocks();
    }

    function _installPrecompileMocks() internal {
        MockAccountKeychain mockKc = new MockAccountKeychain();
        vm.etch(KEYCHAIN, address(mockKc).code);

        MockSciAgentState mockState = new MockSciAgentState();
        vm.etch(SCI_AGENT_STATE, address(mockState).code);
    }

    // -------- Helpers --------

    /// Authorize `keyId` on `account`'s keychain as an unrestricted T3 access key.
    /// Convenience wrapper used by tests that don't care about scope/limit details.
    function authorizeUnrestricted(address account, address keyId) internal {
        IAccountKeychain.TokenLimit[] memory noLimits = new IAccountKeychain.TokenLimit[](0);
        IAccountKeychain.CallScope[] memory noScopes = new IAccountKeychain.CallScope[](0);
        IAccountKeychain.KeyRestrictions memory cfg = IAccountKeychain.KeyRestrictions({
            expiry: uint64(block.timestamp + 1 days),
            enforceLimits: false,
            limits: noLimits,
            allowAnyCalls: true,
            allowedCalls: noScopes
        });
        vm.prank(account);
        IAccountKeychain(KEYCHAIN).authorizeKey(keyId, IAccountKeychain.SignatureType.Secp256k1, cfg);
    }

    /// bytes32 helper that matches `cast format-bytes32-string` semantics — pads
    /// a short ASCII identifier on the RIGHT with zeros (NOT on the left).
    /// Solidity has no built-in for this; the obvious cast `bytes32(bytes(s))`
    /// reverts for strings longer than 32 bytes but otherwise does the right
    /// thing. We keep the wrapper to make test intent clear at the callsite.
    function id(string memory s) internal pure returns (bytes32 out) {
        bytes memory b = bytes(s);
        require(b.length <= 32, "id: string too long");
        assembly {
            out := mload(add(b, 32))
        }
    }
}
