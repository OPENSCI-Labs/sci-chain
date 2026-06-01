// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity ^0.8.20;

import { Test } from "forge-std/Test.sol";

import { ISCIAgentDelegator } from "../src/interfaces/ISCIAgentDelegator.sol";
import { SCIAgentDelegator } from "../src/integration/SCIAgentDelegator.sol";
import { MockAccountKeychain } from "./mocks/MockAccountKeychain.sol";

contract Sink {
    uint256 public last;
    bool public shouldRevert;

    function ping(uint256 v) external payable {
        if (shouldRevert) revert("sink");
        last = v;
    }

    function setShouldRevert(bool v) external {
        shouldRevert = v;
    }
}

contract SCIAgentDelegatorTest is Test {
    address constant KEYCHAIN = 0xaAAAaaAA00000000000000000000000000000000;
    address constant SESSION_KEY = address(0xC0DE);

    SCIAgentDelegator delegator;
    Sink sink;

    function setUp() public {
        MockAccountKeychain mock = new MockAccountKeychain();
        vm.etch(KEYCHAIN, address(mock).code);

        delegator = new SCIAgentDelegator();
        sink = new Sink();
    }

    function _seedHookOk() internal {
        MockAccountKeychain(KEYCHAIN).setTransactionKey(SESSION_KEY);
    }

    function test_RevertWhen_TransactionKeyZero() public {
        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](1);
        calls[0] = ISCIAgentDelegator.Call({
            target: address(sink),
            value: 0,
            data: abi.encodeCall(Sink.ping, (42))
        });

        vm.expectRevert(ISCIAgentDelegator.MissingTransactionKey.selector);
        delegator.execute(calls);
    }

    function test_BatchExecutesInOrder() public {
        _seedHookOk();

        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](2);
        calls[0] = ISCIAgentDelegator.Call({
            target: address(sink),
            value: 0,
            data: abi.encodeCall(Sink.ping, (7))
        });
        calls[1] = ISCIAgentDelegator.Call({
            target: address(sink),
            value: 0,
            data: abi.encodeCall(Sink.ping, (42))
        });

        delegator.execute(calls);
        assertEq(sink.last(), 42);
    }

    function test_RevertWhen_InnerCallReverts() public {
        _seedHookOk();
        sink.setShouldRevert(true);

        ISCIAgentDelegator.Call[] memory calls = new ISCIAgentDelegator.Call[](1);
        calls[0] = ISCIAgentDelegator.Call({
            target: address(sink),
            value: 0,
            data: abi.encodeCall(Sink.ping, (1))
        });

        vm.expectRevert();
        delegator.execute(calls);
    }
}
