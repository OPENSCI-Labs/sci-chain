//! Minimal ERC-20 / ISCI20 interfaces used by the pre-execution hook for selector
//! classification and amount extraction.
//!
//! The hook recognizes these selectors and deducts the configured spending-limit quota
//! from the session key on a pessimistic basis (Q4 R1). `transferFrom` is intentionally
//! not deducted: in that flow the spender (not the session key's root) is the funds source.

crate::sol! {
    #[derive(Debug, PartialEq, Eq)]
    #[sol(abi)]
    interface IERC20 {
        function transfer(address to, uint256 amount) external returns (bool);
        function approve(address spender, uint256 amount) external returns (bool);
        function transferFrom(address from, address to, uint256 amount) external returns (bool);
    }

    /// SCI-Chain attribution extensions on top of standard ERC-20.
    #[derive(Debug, PartialEq, Eq)]
    #[sol(abi)]
    interface ISCI20 {
        function transferWithMemo(address to, uint256 amount, bytes32 memo) external returns (bool);
    }
}
