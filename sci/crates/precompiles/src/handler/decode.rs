//! Pure helper for the pre-execution hook: classifying inner-call selectors into
//! `(token, amount)` pairs for spending-limit accounting.

use alloy_primitives::{Address, U256};
use alloy_sol_types::SolCall;
use tempo_contracts::predeploys::{IERC20, ISCI20};

/// Classifies an inner call's selector for spending-limit purposes.
///
/// Returns `Some((token, amount))` when the call is a recognized token-moving selector
/// whose decoded amount should be deducted from the session key's quota (Q4 R1
/// pessimistic policy). Returns `None` otherwise — scope checks still run independently.
///
/// **Recognized**:
/// - `ERC20.transfer(to, amount)`
/// - `ERC20.approve(spender, amount)` — deducted as a max-commitment, no refund
/// - `ISCI20.transferWithMemo(to, amount, memo)`
///
/// **Intentionally not counted**:
/// - `ERC20.transferFrom(from, to, amount)` — the spender (not the session key's root)
///   is the funds source.
/// - Any other selector — scope check applies but no quota deduction.
pub fn classify_token_call(target: Address, data: &[u8]) -> Option<(Address, U256)> {
    if data.len() < 4 {
        return None;
    }
    let selector: [u8; 4] = data[..4].try_into().expect("len >= 4");

    match selector {
        sel if sel == IERC20::transferCall::SELECTOR => {
            let call = IERC20::transferCall::abi_decode(data).ok()?;
            Some((target, call.amount))
        }
        sel if sel == IERC20::approveCall::SELECTOR => {
            let call = IERC20::approveCall::abi_decode(data).ok()?;
            Some((target, call.amount))
        }
        sel if sel == ISCI20::transferWithMemoCall::SELECTOR => {
            let call = ISCI20::transferWithMemoCall::abi_decode(data).ok()?;
            Some((target, call.amount))
        }
        // transferFrom: spender is msg.sender (not the session key's root in our model);
        // leave quota untouched. Scope still enforced.
        _ => None,
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use alloy_primitives::{Address, B256, U256};
    use alloy_sol_types::SolCall;

    #[test]
    fn classify_erc20_transfer_extracts_amount() {
        let token = Address::repeat_byte(0xaa);
        let to = Address::repeat_byte(0xbb);
        let amount = U256::from(1_000_000u64);
        let data = IERC20::transferCall { to, amount }.abi_encode();

        let (out_token, out_amount) = classify_token_call(token, &data).expect("classified");
        assert_eq!(out_token, token);
        assert_eq!(out_amount, amount);
    }

    #[test]
    fn classify_erc20_approve_extracts_amount_pessimistically() {
        let token = Address::repeat_byte(0xaa);
        let spender = Address::repeat_byte(0xcc);
        let amount = U256::from(u128::MAX);
        let data = IERC20::approveCall { spender, amount }.abi_encode();

        let (out_token, out_amount) = classify_token_call(token, &data).expect("classified");
        assert_eq!(out_token, token);
        assert_eq!(out_amount, amount, "full approve amount, no max-min capping");
    }

    #[test]
    fn classify_sci20_transfer_with_memo_extracts_amount() {
        let token = Address::repeat_byte(0xaa);
        let to = Address::repeat_byte(0xbb);
        let amount = U256::from(50u64);
        let memo = B256::repeat_byte(0xff);
        let data = ISCI20::transferWithMemoCall { to, amount, memo }.abi_encode();

        let (out_token, out_amount) = classify_token_call(token, &data).expect("classified");
        assert_eq!(out_token, token);
        assert_eq!(out_amount, amount);
    }

    #[test]
    fn classify_transferfrom_returns_none() {
        let token = Address::repeat_byte(0xaa);
        let data = IERC20::transferFromCall {
            from: Address::repeat_byte(0x11),
            to: Address::repeat_byte(0x22),
            amount: U256::from(100u64),
        }
        .abi_encode();

        assert!(classify_token_call(token, &data).is_none());
    }

    #[test]
    fn classify_unknown_selector_returns_none() {
        let token = Address::repeat_byte(0xaa);
        let data = vec![0x12, 0x34, 0x56, 0x78, 0xff];
        assert!(classify_token_call(token, &data).is_none());
    }

    #[test]
    fn classify_short_calldata_returns_none() {
        assert!(classify_token_call(Address::ZERO, &[]).is_none());
        assert!(classify_token_call(Address::ZERO, &[0x12, 0x34]).is_none());
    }
}
