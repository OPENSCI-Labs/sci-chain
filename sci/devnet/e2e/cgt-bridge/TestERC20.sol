// SPDX-License-Identifier: MIT
pragma solidity ^0.8.15;

/// @title TestERC20
/// @notice Minimal dependency-free ERC-20 used to e2e-test the CGT v2 ETH->L2
///         bridge path on devnet. Constructor mints 1,000,000 tokens to the
///         deployer. Stands in for an L1 asset (e.g. WETH) to bridge to L2 as
///         an OptimismMintableERC20.
contract TestERC20 {
    string public name = "Bridged ETH (test)";
    string public symbol = "bETH";
    uint8 public decimals = 18;
    uint256 public totalSupply;
    mapping(address => uint256) public balanceOf;
    mapping(address => mapping(address => uint256)) public allowance;
    event Transfer(address indexed from, address indexed to, uint256 value);
    event Approval(address indexed owner, address indexed spender, uint256 value);

    constructor() {
        totalSupply = 1_000_000 ether;
        balanceOf[msg.sender] = totalSupply;
        emit Transfer(address(0), msg.sender, totalSupply);
    }

    function transfer(address to, uint256 v) external returns (bool) {
        balanceOf[msg.sender] -= v;
        balanceOf[to] += v;
        emit Transfer(msg.sender, to, v);
        return true;
    }

    function approve(address s, uint256 v) external returns (bool) {
        allowance[msg.sender][s] = v;
        emit Approval(msg.sender, s, v);
        return true;
    }

    function transferFrom(address f, address to, uint256 v) external returns (bool) {
        if (allowance[f][msg.sender] != type(uint256).max) allowance[f][msg.sender] -= v;
        balanceOf[f] -= v;
        balanceOf[to] += v;
        emit Transfer(f, to, v);
        return true;
    }
}
