//! Dev tool: build, sign, and 2718-encode a SCI AA transaction (type `0x76`).
//!
//! `cast` cannot construct a custom transaction type, so this helper reuses the exact
//! node encoding (`base-common-consensus`) to produce a raw tx ready for
//! `eth_sendRawTransaction`.
//!
//! Usage:
//!   aa-txgen <priv_key_hex> <chain_id> <nonce> <to_addr> <value_wei>
//!
//! Optional environment overrides:
//!   MAX_FEE      max_fee_per_gas          (default 1_000_000_000)
//!   MAX_PRIO     max_priority_fee_per_gas (default 1_000_000)
//!   GAS_LIMIT    gas_limit                (default 100_000)
//!   INPUT        first-call calldata hex  (default empty)
//!   FEE_PAYER    fee_payer address        (default none)
//!   ROOT         root account calls run as (default none = run as signer)
//!   CALL2_TO     second call target       (adds a 2nd call to exercise batch)
//!   CALL2_VALUE  second call value_wei    (default 0 when CALL2_TO set)

use std::{env, str::FromStr};

use alloy_consensus::{SignableTransaction, Signed};
use alloy_eips::{eip2718::Encodable2718, eip2930::AccessList};
use alloy_primitives::{Address, Bytes, TxKind, U256, hex};
use alloy_signer::SignerSync;
use alloy_signer_local::PrivateKeySigner;
use base_common_consensus::{BaseAaTransaction, BaseTxEnvelope, Call};

/// Reads an environment override, falling back to `default` when unset.
fn env_or<T: FromStr>(key: &str, default: T) -> T {
    env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 6 {
        eprintln!(
            "usage: aa-txgen <priv_key_hex> <chain_id> <nonce> <to_addr> <value_wei>\n\
             (optional env: MAX_FEE MAX_PRIO GAS_LIMIT INPUT FEE_PAYER CALL2_TO CALL2_VALUE)"
        );
        std::process::exit(2);
    }

    let signer =
        PrivateKeySigner::from_str(args[1].trim_start_matches("0x")).expect("invalid private key");
    let chain_id: u64 = args[2].parse().expect("invalid chain_id");
    let nonce: u64 = args[3].parse().expect("invalid nonce");
    let to = Address::from_str(&args[4]).expect("invalid to address");
    let value = U256::from_str(&args[5]).expect("invalid value");

    let input = env::var("INPUT")
        .ok()
        .map(|h| Bytes::from(hex::decode(h.trim_start_matches("0x")).expect("invalid INPUT hex")))
        .unwrap_or_default();

    let mut calls = vec![Call { to: TxKind::Call(to), value, input }];
    if let Ok(call2_to) = env::var("CALL2_TO") {
        let call2 = Address::from_str(&call2_to).expect("invalid CALL2_TO");
        let call2_value = U256::from_str(&env::var("CALL2_VALUE").unwrap_or_else(|_| "0".into()))
            .expect("invalid CALL2_VALUE");
        calls.push(Call { to: TxKind::Call(call2), value: call2_value, input: Bytes::new() });
    }

    let fee_payer =
        env::var("FEE_PAYER").ok().map(|a| Address::from_str(&a).expect("invalid FEE_PAYER"));

    let root = env::var("ROOT").ok().map(|a| Address::from_str(&a).expect("invalid ROOT"));

    let tx = BaseAaTransaction {
        chain_id,
        nonce,
        max_priority_fee_per_gas: env_or("MAX_PRIO", 1_000_000u128),
        max_fee_per_gas: env_or("MAX_FEE", 1_000_000_000u128),
        gas_limit: env_or("GAS_LIMIT", 100_000u64),
        calls,
        access_list: AccessList::default(),
        fee_payer,
        root,
    };

    let signature = signer.sign_hash_sync(&tx.signature_hash()).expect("signing failed");
    let envelope = BaseTxEnvelope::Aa(Signed::new_unhashed(tx, signature));

    let mut raw = Vec::new();
    envelope.encode_2718(&mut raw);

    eprintln!("signer  : {}", signer.address());
    eprintln!("tx_hash : {}", envelope.tx_hash());
    eprintln!("type    : 0x{:02x}", raw[0]);
    println!("{}", hex::encode_prefixed(&raw));
}
