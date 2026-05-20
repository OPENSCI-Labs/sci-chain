//! Pure helpers for the pre-execution hook: decoding an `execute(Call[])` batch and
//! classifying inner-call selectors into `(token, amount)` pairs for spending-limit
//! accounting.

use alloy_primitives::{Address, U256};
use alloy_sol_types::SolCall;
use tempo_contracts::predeploys::{IERC20, ISCI20, ISCIAgentDelegator};

/// One inner call inside a 7702 batch — the shape the hook iterates over after decoding
/// `execute(Call[])`. We carry `target` as a plain [`Address`] (rather than `TxKind`)
/// because the decoded batch never encodes `CREATE`; the keychain scope check still
/// receives `TxKind::Call(target)` to share the existing implementation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InnerCall {
    /// Call target (always a concrete address; batch entries can't be CREATE).
    pub target: Address,
    /// Wei value forwarded to the inner call.
    pub value: U256,
    /// Calldata forwarded to the inner call.
    pub data: Vec<u8>,
}

/// Decodes `tx.input` as `SCIAgentDelegator.execute(Call[])`.
///
/// Returns `None` when the calldata is not the canonical `execute(Call[])` selector +
/// payload (e.g. shorter than 4 bytes, unknown selector, or malformed ABI). The hook
/// caller falls back to a single-call probe in that case (Q2 fallback path).
pub fn decode_execute_batch(input: &[u8]) -> Option<Vec<InnerCall>> {
    if input.len() < 4 {
        return None;
    }
    let selector: [u8; 4] = input[..4].try_into().expect("len >= 4");
    if selector != ISCIAgentDelegator::executeCall::SELECTOR {
        return None;
    }
    let decoded = ISCIAgentDelegator::executeCall::abi_decode(input).ok()?;
    Some(
        decoded
            .calls
            .into_iter()
            .map(|c| InnerCall {
                target: c.target,
                value: c.value,
                data: c.data.to_vec(),
            })
            .collect(),
    )
}

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
    use alloy_primitives::{Address, B256, Bytes, U256};
    use alloy_sol_types::SolCall;
    use tempo_contracts::predeploys::ISCIAgentDelegator::Call;

    fn make_call(target: Address, value: U256, data: Vec<u8>) -> Call {
        Call {
            target,
            value,
            data: Bytes::from(data),
        }
    }

    #[test]
    fn decodes_execute_batch_of_three() {
        let calls = vec![
            make_call(Address::repeat_byte(1), U256::ZERO, vec![0xde, 0xad, 0xbe, 0xef]),
            make_call(Address::repeat_byte(2), U256::from(42u64), vec![1, 2, 3]),
            make_call(Address::repeat_byte(3), U256::ZERO, vec![]),
        ];
        let encoded = ISCIAgentDelegator::executeCall { calls }.abi_encode();

        let decoded = decode_execute_batch(&encoded).expect("decode should succeed");
        assert_eq!(decoded.len(), 3);
        assert_eq!(decoded[0].target, Address::repeat_byte(1));
        assert_eq!(decoded[1].value, U256::from(42u64));
        assert_eq!(decoded[2].data, Vec::<u8>::new());
    }

    #[test]
    fn decodes_empty_execute_batch() {
        let encoded = ISCIAgentDelegator::executeCall { calls: vec![] }.abi_encode();
        let decoded = decode_execute_batch(&encoded).expect("decode empty");
        assert!(decoded.is_empty());
    }

    #[test]
    fn unknown_outer_selector_returns_none() {
        let data = vec![0x12, 0x34, 0x56, 0x78, 0xff];
        assert!(decode_execute_batch(&data).is_none());
    }

    #[test]
    fn short_input_returns_none() {
        assert!(decode_execute_batch(&[]).is_none());
        assert!(decode_execute_batch(&[0u8]).is_none());
        assert!(decode_execute_batch(&[0u8, 0, 0]).is_none());
    }

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
