//! SCI account-abstraction (AA) transaction type — Plan A PoC.
//!
//! A minimal native agent transaction tagged with type id `0x76` (matching Tempo's AA
//! tx tag). It carries a batch of [`Call`]s and an optional `fee_payer` (the Plan A
//! "sponsored gas" field: gas can be charged to the fee payer / root rather than the
//! sending session key). This PoC is intentionally minimal — it does NOT yet carry the
//! keychain signature, 2D nonce, validity windows, or authorization list that the full
//! Tempo AA tx has (those land in Phase 1). It is signed with a standard secp256k1
//! [`Signature`] so it can ride the existing envelope/signing plumbing while we validate
//! that a brand-new tx type flows through decode -> execution -> proof (the Go/No-Go gate).

use alloc::vec::Vec;
use core::mem;

use alloy_consensus::{
    InMemorySize, SignableTransaction, Transaction, Typed2718,
    transaction::{RlpEcdsaDecodableTx, RlpEcdsaEncodableTx},
};
use alloy_eips::eip2930::AccessList;
use alloy_primitives::{Address, B256, Bytes, ChainId, Signature, TxKind, U256, bytes::BufMut};
use alloy_rlp::{Decodable, Encodable, Header, RlpDecodable, RlpEncodable};

/// SCI AA transaction type id (`0x76`), matching Tempo's AA tx tag.
pub const SCI_AA_TX_TYPE_ID: u8 = 0x76;

/// One inner call inside an AA batch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, RlpEncodable, RlpDecodable)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct Call {
    /// Call target (or CREATE).
    pub to: TxKind,
    /// Wei value forwarded to the call.
    pub value: U256,
    /// Calldata forwarded to the call.
    pub input: Bytes,
}

impl Call {
    /// Heuristic in-memory size.
    pub fn size(&self) -> usize {
        mem::size_of::<TxKind>() + mem::size_of::<U256>() + self.input.len()
    }
}

/// SCI account-abstraction transaction (PoC, type `0x76`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "arbitrary", derive(arbitrary::Arbitrary))]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
pub struct BaseAaTransaction {
    /// Chain ID.
    pub chain_id: ChainId,
    /// Sender nonce.
    pub nonce: u64,
    /// EIP-1559 max priority fee per gas.
    pub max_priority_fee_per_gas: u128,
    /// EIP-1559 max fee per gas.
    pub max_fee_per_gas: u128,
    /// Gas limit.
    pub gas_limit: u64,
    /// Batch of calls executed atomically.
    pub calls: Vec<Call>,
    /// EIP-2930 access list.
    pub access_list: AccessList,
    /// Optional fee payer (sponsored gas). `None` means the sender pays gas.
    pub fee_payer: Option<Address>,
}

impl BaseAaTransaction {
    /// Returns the transaction type id.
    #[doc(alias = "transaction_type")]
    pub const fn tx_type() -> u8 {
        SCI_AA_TX_TYPE_ID
    }

    /// Heuristic in-memory size.
    pub fn size(&self) -> usize {
        mem::size_of::<Self>()
            + self.calls.iter().map(Call::size).sum::<usize>()
            + self.access_list.size()
    }

    /// RLP length of the `fee_payer` field (`Some(addr)` encodes the address, `None`
    /// encodes an empty string).
    fn fee_payer_rlp_len(&self) -> usize {
        self.fee_payer.map_or(1, |addr| addr.length())
    }

    fn encode_fee_payer(&self, out: &mut dyn BufMut) {
        match self.fee_payer {
            Some(addr) => addr.encode(out),
            None => out.put_u8(alloy_rlp::EMPTY_STRING_CODE),
        }
    }

    fn decode_fee_payer(buf: &mut &[u8]) -> alloy_rlp::Result<Option<Address>> {
        // Peek: an empty string (0x80) means `None`; otherwise decode an address.
        if buf.first().copied() == Some(alloy_rlp::EMPTY_STRING_CODE) {
            *buf = &buf[1..];
            Ok(None)
        } else {
            Ok(Some(Address::decode(buf)?))
        }
    }
}

impl RlpEcdsaEncodableTx for BaseAaTransaction {
    fn rlp_encoded_fields_length(&self) -> usize {
        self.chain_id.length()
            + self.nonce.length()
            + self.max_priority_fee_per_gas.length()
            + self.max_fee_per_gas.length()
            + self.gas_limit.length()
            + self.calls.length()
            + self.access_list.length()
            + self.fee_payer_rlp_len()
    }

    fn rlp_encode_fields(&self, out: &mut dyn BufMut) {
        self.chain_id.encode(out);
        self.nonce.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        self.calls.encode(out);
        self.access_list.encode(out);
        self.encode_fee_payer(out);
    }
}

impl RlpEcdsaDecodableTx for BaseAaTransaction {
    const DEFAULT_TX_TYPE: u8 = SCI_AA_TX_TYPE_ID;

    fn rlp_decode_fields(buf: &mut &[u8]) -> alloy_rlp::Result<Self> {
        Ok(Self {
            chain_id: Decodable::decode(buf)?,
            nonce: Decodable::decode(buf)?,
            max_priority_fee_per_gas: Decodable::decode(buf)?,
            max_fee_per_gas: Decodable::decode(buf)?,
            gas_limit: Decodable::decode(buf)?,
            calls: Decodable::decode(buf)?,
            access_list: Decodable::decode(buf)?,
            fee_payer: Self::decode_fee_payer(buf)?,
        })
    }
}

impl Transaction for BaseAaTransaction {
    fn chain_id(&self) -> Option<ChainId> {
        Some(self.chain_id)
    }

    fn nonce(&self) -> u64 {
        self.nonce
    }

    fn gas_limit(&self) -> u64 {
        self.gas_limit
    }

    fn gas_price(&self) -> Option<u128> {
        None
    }

    fn max_fee_per_gas(&self) -> u128 {
        self.max_fee_per_gas
    }

    fn max_priority_fee_per_gas(&self) -> Option<u128> {
        Some(self.max_priority_fee_per_gas)
    }

    fn max_fee_per_blob_gas(&self) -> Option<u128> {
        None
    }

    fn priority_fee_or_price(&self) -> u128 {
        self.max_priority_fee_per_gas
    }

    fn effective_gas_price(&self, base_fee: Option<u64>) -> u128 {
        alloy_eips::eip1559::calc_effective_gas_price(
            self.max_fee_per_gas,
            self.max_priority_fee_per_gas,
            base_fee,
        )
    }

    fn is_dynamic_fee(&self) -> bool {
        true
    }

    fn kind(&self) -> TxKind {
        self.calls.first().map(|c| c.to).unwrap_or(TxKind::Create)
    }

    fn is_create(&self) -> bool {
        self.kind().is_create()
    }

    fn value(&self) -> U256 {
        self.calls.iter().fold(U256::ZERO, |acc, call| acc.saturating_add(call.value))
    }

    fn input(&self) -> &Bytes {
        static EMPTY_BYTES: Bytes = Bytes::new();
        self.calls.first().map(|c| &c.input).unwrap_or(&EMPTY_BYTES)
    }

    fn access_list(&self) -> Option<&AccessList> {
        Some(&self.access_list)
    }

    fn blob_versioned_hashes(&self) -> Option<&[B256]> {
        None
    }

    fn authorization_list(&self) -> Option<&[alloy_eips::eip7702::SignedAuthorization]> {
        None
    }
}

impl Typed2718 for BaseAaTransaction {
    fn ty(&self) -> u8 {
        SCI_AA_TX_TYPE_ID
    }
}

impl SignableTransaction<Signature> for BaseAaTransaction {
    fn set_chain_id(&mut self, chain_id: ChainId) {
        self.chain_id = chain_id;
    }

    fn encode_for_signing(&self, out: &mut dyn BufMut) {
        out.put_u8(Self::tx_type());
        let payload_length = self.rlp_encoded_fields_length();
        Header { list: true, payload_length }.encode(out);
        self.rlp_encode_fields(out);
    }

    fn payload_len_for_signature(&self) -> usize {
        let payload_length = self.rlp_encoded_fields_length();
        1 + Header { list: true, payload_length }.length_with_payload()
    }
}

impl InMemorySize for BaseAaTransaction {
    fn size(&self) -> usize {
        BaseAaTransaction::size(self)
    }
}

#[cfg(test)]
mod tests {
    use alloy_consensus::transaction::{RlpEcdsaDecodableTx, RlpEcdsaEncodableTx};
    use alloy_primitives::{Address, U256, address};
    use alloy_rlp::{Decodable, Encodable};

    use super::*;

    fn sample() -> BaseAaTransaction {
        BaseAaTransaction {
            chain_id: 42001,
            nonce: 7,
            max_priority_fee_per_gas: 1_000_000_000,
            max_fee_per_gas: 2_000_000_000,
            gas_limit: 210_000,
            calls: alloc::vec![
                Call {
                    to: TxKind::Call(address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8")),
                    value: U256::from(1u64),
                    input: Bytes::from_static(&[0xde, 0xad, 0xbe, 0xef]),
                },
                Call {
                    to: TxKind::Call(Address::repeat_byte(0x11)),
                    value: U256::ZERO,
                    input: Bytes::new(),
                },
            ],
            access_list: AccessList::default(),
            fee_payer: Some(address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266")),
        }
    }

    #[test]
    fn rlp_fields_roundtrip_with_fee_payer() {
        let tx = sample();
        let mut buf = Vec::new();
        tx.rlp_encode_fields(&mut buf);
        assert_eq!(buf.len(), tx.rlp_encoded_fields_length());

        let mut slice = buf.as_slice();
        let decoded = BaseAaTransaction::rlp_decode_fields(&mut slice).unwrap();
        assert!(slice.is_empty());
        assert_eq!(decoded, tx);
    }

    #[test]
    fn rlp_fields_roundtrip_without_fee_payer() {
        let mut tx = sample();
        tx.fee_payer = None;
        let mut buf = Vec::new();
        tx.rlp_encode_fields(&mut buf);
        assert_eq!(buf.len(), tx.rlp_encoded_fields_length());

        let mut slice = buf.as_slice();
        let decoded = BaseAaTransaction::rlp_decode_fields(&mut slice).unwrap();
        assert!(slice.is_empty());
        assert_eq!(decoded, tx);
        assert_eq!(decoded.fee_payer, None);
    }

    #[test]
    fn call_rlp_roundtrip() {
        let call = Call {
            to: TxKind::Call(Address::repeat_byte(0x22)),
            value: U256::from(99u64),
            input: Bytes::from_static(&[1, 2, 3]),
        };
        let mut buf = Vec::new();
        call.encode(&mut buf);
        let decoded = Call::decode(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded, call);
    }

    #[test]
    fn ty_is_0x76() {
        assert_eq!(BaseAaTransaction::tx_type(), 0x76);
        assert_eq!(Typed2718::ty(&sample()), 0x76);
    }
}
