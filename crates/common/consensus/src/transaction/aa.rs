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
    InMemorySize, SignableTransaction, Transaction, TxEip1559, Typed2718,
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
    /// Optional root account the calls execute on behalf of (Plan A identity model).
    ///
    /// The transaction is signed by the **session key**; when `root` is `Some(addr)`, the
    /// inner calls execute with `msg.sender == addr` after the keychain authorizes
    /// `keys[root][session_key]`. `None` means a plain batch executed as the signer itself
    /// (no keychain root-delegation).
    pub root: Option<Address>,
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

    /// PoC helper: approximate this AA transaction as an EIP-1559 transaction that
    /// executes only its first call. Used to reuse the existing `TxEnv` /
    /// `TransactionRequest` conversions until the Tempo batch handler lands (Phase 2).
    /// Drops `fee_payer` and any calls beyond the first — single-call execution only.
    pub fn to_eip1559_first_call(&self) -> TxEip1559 {
        let (to, value, input) = self
            .calls
            .first()
            .map(|c| (c.to, c.value, c.input.clone()))
            .unwrap_or((TxKind::Create, U256::ZERO, Bytes::new()));
        TxEip1559 {
            chain_id: self.chain_id,
            nonce: self.nonce,
            gas_limit: self.gas_limit,
            max_fee_per_gas: self.max_fee_per_gas,
            max_priority_fee_per_gas: self.max_priority_fee_per_gas,
            to,
            value,
            access_list: self.access_list.clone(),
            input,
        }
    }

    /// RLP length of an optional address field (`Some(addr)` encodes the address, `None`
    /// encodes an empty string).
    fn opt_address_rlp_len(addr: Option<Address>) -> usize {
        addr.map_or(1, |a| a.length())
    }

    fn encode_opt_address(addr: Option<Address>, out: &mut dyn BufMut) {
        match addr {
            Some(a) => a.encode(out),
            None => out.put_u8(alloy_rlp::EMPTY_STRING_CODE),
        }
    }

    fn decode_opt_address(buf: &mut &[u8]) -> alloy_rlp::Result<Option<Address>> {
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
            + Self::opt_address_rlp_len(self.fee_payer)
            + Self::opt_address_rlp_len(self.root)
    }

    fn rlp_encode_fields(&self, out: &mut dyn BufMut) {
        self.chain_id.encode(out);
        self.nonce.encode(out);
        self.max_priority_fee_per_gas.encode(out);
        self.max_fee_per_gas.encode(out);
        self.gas_limit.encode(out);
        self.calls.encode(out);
        self.access_list.encode(out);
        Self::encode_opt_address(self.fee_payer, out);
        Self::encode_opt_address(self.root, out);
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
            fee_payer: Self::decode_opt_address(buf)?,
            root: Self::decode_opt_address(buf)?,
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
            root: Some(Address::repeat_byte(0x99)),
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
        tx.root = None;
        let mut buf = Vec::new();
        tx.rlp_encode_fields(&mut buf);
        assert_eq!(buf.len(), tx.rlp_encoded_fields_length());

        let mut slice = buf.as_slice();
        let decoded = BaseAaTransaction::rlp_decode_fields(&mut slice).unwrap();
        assert!(slice.is_empty());
        assert_eq!(decoded, tx);
        assert_eq!(decoded.fee_payer, None);
        assert_eq!(decoded.root, None);
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

    /// The gate-relevant property: an AA tx wrapped in [`BaseTxEnvelope`] survives a
    /// full EIP-2718 encode -> decode round-trip (this is the exact path the proof
    /// client uses via `BaseTxEnvelope::decode_2718`).
    #[test]
    fn envelope_2718_roundtrip_aa() {
        use alloy_consensus::Signed;
        use alloy_eips::eip2718::{Decodable2718, Encodable2718};

        use crate::{BaseTxEnvelope, OpTxType};

        let tx = sample();
        let sig = Signature::new(U256::from(1u64), U256::from(2u64), false);
        let envelope = BaseTxEnvelope::Aa(Signed::new_unhashed(tx.clone(), sig));

        let mut buf = Vec::new();
        envelope.encode_2718(&mut buf);
        assert_eq!(buf[0], super::SCI_AA_TX_TYPE_ID, "type byte must be 0x76");

        let decoded = BaseTxEnvelope::decode_2718(&mut buf.as_slice()).unwrap();
        assert_eq!(decoded.tx_type(), OpTxType::Aa);
        match decoded {
            BaseTxEnvelope::Aa(signed) => assert_eq!(signed.tx(), &tx),
            other => panic!("expected Aa variant, got {other:?}"),
        }
    }
}

/// Milestone C: the execution-conversion path the proof executor relies on.
///
/// The proof client / executor are generic over `F::Tx: FromRecoveredTx<BaseTxEnvelope>`
/// (see `crates/proof/client/{driver,prologue}.rs`). This test exercises exactly that
/// conversion for the new AA variant and asserts the produced [`TxEnv`] matches the
/// AA tx's first call (the PoC single-call execution semantics), proving the proof
/// executor will build the correct EVM environment for an AA transaction.
#[cfg(all(test, feature = "evm"))]
mod evm_tests {
    use alloy_consensus::Signed;
    use alloy_evm::FromRecoveredTx;
    use alloy_primitives::{Signature, address};
    use revm::context::TxEnv;

    use super::{BaseAaTransaction, Bytes, Call, TxKind, U256};
    use crate::BaseTxEnvelope;

    #[test]
    fn aa_txenv_matches_first_call_eip1559() {
        let caller = address!("0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266");
        let recipient = address!("0x70997970C51812dc3A010C7d01b50e0d17dc79C8");
        let aa = BaseAaTransaction {
            chain_id: 42001,
            nonce: 5,
            max_priority_fee_per_gas: 1,
            max_fee_per_gas: 100,
            gas_limit: 50_000,
            calls: alloc::vec![Call {
                to: TxKind::Call(recipient),
                value: U256::from(777u64),
                input: Bytes::from_static(&[0xaa, 0xbb]),
            }],
            access_list: Default::default(),
            fee_payer: None,
            root: None,
        };
        let sig = Signature::new(U256::from(1u64), U256::from(2u64), false);

        // The conversion the proof executor uses: FromRecoveredTx<BaseTxEnvelope>.
        let aa_env = BaseTxEnvelope::Aa(Signed::new_unhashed(aa.clone(), sig));
        let tx_env_aa = TxEnv::from_recovered_tx(&aa_env, caller);

        // The equivalent EIP-1559 envelope built from the AA tx's first call.
        let eip_env =
            BaseTxEnvelope::Eip1559(Signed::new_unhashed(aa.to_eip1559_first_call(), sig));
        let tx_env_eip = TxEnv::from_recovered_tx(&eip_env, caller);

        assert_eq!(tx_env_aa, tx_env_eip, "AA TxEnv must equal the first-call EIP-1559 TxEnv");

        // And it reflects the AA tx's first call + top-level fields.
        assert_eq!(tx_env_aa.caller, caller);
        assert_eq!(tx_env_aa.kind, TxKind::Call(recipient));
        assert_eq!(tx_env_aa.value, U256::from(777u64));
        assert_eq!(tx_env_aa.data, Bytes::from_static(&[0xaa, 0xbb]));
        assert_eq!(tx_env_aa.nonce, 5);
        assert_eq!(tx_env_aa.chain_id, Some(42001));
    }
}
