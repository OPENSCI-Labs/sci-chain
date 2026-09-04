//! Minimal ERC-20 interface — only the selectors the `AccountKeychain` uses to identify
//! recipient-constrained token calls.

crate::sol! {
    #[derive(Debug, PartialEq, Eq)]
    #[sol(abi)]
    interface ITIP20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferWithMemo(address to, uint256 amount, bytes32 memo) external;
    }
}
