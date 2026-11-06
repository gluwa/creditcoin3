//! `OrderedBlock::try_from_fetched_block` must reject receipts that belong to a different block
//! than the one fetched.
//!
//! `get_block` and `get_block_receipts` are two separate height-keyed calls, and a provider URL is
//! commonly a load balancer over many nodes, so the pair can be answered by peers on opposite
//! sides of a reorg. These tests pin the hash comparison that catches it, including the two cases
//! it must *not* reject: receipts that omit the optional `blockHash`, and the empty-block path
//! that skips the header-root check entirely.

use alloy::network::{AnyRpcBlock, AnyTransactionReceipt};
use eth::{ChainFamily, Error, OrderedBlock};
use serde_json::{json, Value};
use usc_abi_encoding::common::EncodingVersion;

const ZERO32: &str = "0x0000000000000000000000000000000000000000000000000000000000000000";
const BLOCK_HASH: &str = "0x1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_HASH: &str = "0x2222222222222222222222222222222222222222222222222222222222222222";
const TX_HASH: &str = "0x3333333333333333333333333333333333333333333333333333333333333333";
const ADDR: &str = "0x0000000000000000000000000000000000000001";

fn block_json(hash: &str, number: u64, with_tx: bool) -> Value {
    let transactions = if with_tx {
        json!([{
            "hash": TX_HASH, "nonce": "0x0", "blockHash": hash,
            "blockNumber": format!("{number:#x}"), "transactionIndex": "0x0",
            "from": ADDR, "to": ADDR, "value": "0x0", "gas": "0x5208",
            "gasPrice": "0x1", "input": "0x", "type": "0x0", "chainId": "0x1",
            "v": "0x1b", "r": ZERO32, "s": ZERO32,
        }])
    } else {
        json!([])
    };
    json!({
        "hash": hash, "parentHash": ZERO32, "sha3Uncles": ZERO32, "miner": ADDR,
        "stateRoot": ZERO32, "transactionsRoot": ZERO32, "receiptsRoot": ZERO32,
        "logsBloom": format!("0x{}", "00".repeat(256)),
        "difficulty": "0x0", "number": format!("{number:#x}"), "gasLimit": "0x1c9c380",
        "gasUsed": "0x5208", "timestamp": "0x64", "extraData": "0x", "mixHash": ZERO32,
        "nonce": "0x0000000000000000", "baseFeePerGas": "0x1",
        "transactions": transactions, "uncles": []
    })
}

/// `block_hash` is `Option` in the RPC schema; `None` here means the field is omitted.
fn receipt_json(block_hash: Option<&str>, number: u64) -> Value {
    let mut receipt = json!({
        "transactionHash": TX_HASH, "transactionIndex": "0x0",
        "blockNumber": format!("{number:#x}"), "from": ADDR, "to": ADDR,
        "cumulativeGasUsed": "0x5208", "gasUsed": "0x5208", "contractAddress": null,
        "logs": [], "logsBloom": format!("0x{}", "00".repeat(256)),
        "status": "0x1", "type": "0x0", "effectiveGasPrice": "0x1",
    });
    if let Some(h) = block_hash {
        receipt["blockHash"] = json!(h);
    }
    receipt
}

fn block(hash: &str, number: u64, with_tx: bool) -> AnyRpcBlock {
    serde_json::from_value(block_json(hash, number, with_tx)).expect("block fixture")
}

fn receipts(block_hash: Option<&str>, number: u64) -> Vec<AnyTransactionReceipt> {
    vec![serde_json::from_value(receipt_json(block_hash, number)).expect("receipt fixture")]
}

#[test]
fn receipts_from_another_block_are_rejected() {
    let err = OrderedBlock::try_from_fetched_block(
        1,
        ChainFamily::Ethereum,
        block(BLOCK_HASH, 7, true),
        receipts(Some(OTHER_HASH), 7),
        7,
        EncodingVersion::V1,
    )
    .expect_err("receipts belonging to another block must not be accepted");

    match err {
        Error::ReceiptsBlockMismatch {
            number,
            block,
            receipts,
        } => {
            assert_eq!(number, 7);
            assert_eq!(format!("{block:?}").to_lowercase(), BLOCK_HASH);
            assert_eq!(format!("{receipts:?}").to_lowercase(), OTHER_HASH);
        }
        other => panic!("expected ReceiptsBlockMismatch, got {other:?}"),
    }

    // It must reach the fallback walk, not the reconnect path.
    assert!(Error::ReceiptsBlockMismatch {
        number: 7,
        block: BLOCK_HASH.parse().unwrap(),
        receipts: OTHER_HASH.parse().unwrap(),
    }
    .inconsistent_block_payload_for_fallback());
    assert_eq!(
        Error::ReceiptsBlockMismatch {
            number: 7,
            block: BLOCK_HASH.parse().unwrap(),
            receipts: OTHER_HASH.parse().unwrap(),
        }
        .inconsistent_block_number_hint(),
        Some(7)
    );
}

#[test]
fn matching_receipts_pass_the_hash_check() {
    // The synthetic fixture has placeholder header roots, so this still fails further down at the
    // root comparison. The point is that it gets past the hash check rather than being rejected
    // by it.
    let err = OrderedBlock::try_from_fetched_block(
        1,
        ChainFamily::Ethereum,
        block(BLOCK_HASH, 7, true),
        receipts(Some(BLOCK_HASH), 7),
        7,
        EncodingVersion::V1,
    )
    .expect_err("placeholder roots cannot reproduce");
    assert!(
        !matches!(err, Error::ReceiptsBlockMismatch { .. }),
        "matching hashes must not trip the mismatch check, got {err:?}"
    );
}

#[test]
fn receipts_without_a_block_hash_are_not_rejected() {
    // Optional in the RPC schema and omitted by some providers. Those payloads must fall through
    // to the header-root check exactly as they did before, not be treated as a mismatch.
    let err = OrderedBlock::try_from_fetched_block(
        1,
        ChainFamily::Ethereum,
        block(BLOCK_HASH, 7, true),
        receipts(None, 7),
        7,
        EncodingVersion::V1,
    )
    .expect_err("placeholder roots cannot reproduce");
    assert!(
        !matches!(err, Error::ReceiptsBlockMismatch { .. }),
        "an absent block_hash is unverifiable, not a mismatch, got {err:?}"
    );
}

#[test]
fn empty_blocks_still_skip_the_root_check() {
    // The empty-block carve-out is load-bearing for Substrate and Frontier dev chains. There are
    // no receipts to compare, so the new check must leave it untouched.
    let ordered = OrderedBlock::try_from_fetched_block(
        1,
        ChainFamily::Ethereum,
        block(BLOCK_HASH, 7, false),
        vec![],
        7,
        EncodingVersion::V1,
    )
    .expect("an empty block with no receipts stays acceptable");
    assert_eq!(ordered.number(), 7);
    assert!(ordered.items().is_empty());
}
