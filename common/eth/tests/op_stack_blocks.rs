use alloy::{
    consensus::proofs::ordered_trie_root_with_encoder,
    network::{AnyRpcBlock, AnyTransactionReceipt},
    primitives::{keccak256, B256},
};
use eth::{
    op_stack::{encode_deposit_receipt_2718, DepositError, DepositReceiptFields},
    ChainFamily, DepositTransaction, Error, OrderedBlock, TxRx,
};
use serde_json::{json, Value};
use usc_abi_encoding::common::EncodingVersion;
use utils::block_item_traits::BlockItem;

fn fixture(historical: bool) -> (Value, Value) {
    let (block, receipts) = if historical {
        (
            include_str!("fixtures/op_mainnet_110000000_block.json"),
            include_str!("fixtures/op_mainnet_110000000_receipts.json"),
        )
    } else {
        (
            include_str!("fixtures/base_sepolia_46388021_block.json"),
            include_str!("fixtures/base_sepolia_46388021_receipts.json"),
        )
    };
    (
        serde_json::from_str(block).unwrap(),
        serde_json::from_str(receipts).unwrap(),
    )
}

fn ordered(block: Value, receipts: Value, chain_id: u64) -> Result<OrderedBlock, Error> {
    let block: AnyRpcBlock = serde_json::from_value(block).unwrap();
    let height = block.header.number;
    let receipts: Vec<AnyTransactionReceipt> = serde_json::from_value(receipts).unwrap();
    OrderedBlock::try_from_fetched_block(
        chain_id,
        ChainFamily::OpStack,
        block,
        receipts,
        height,
        EncodingVersion::V1,
    )
}

#[test]
fn canyon_missing_rpc_nonce_uses_the_verified_receipt_nonce() {
    let (mut block, receipts) = fixture(false);
    let canonical = ordered(block.clone(), receipts.clone(), 84532).unwrap();
    assert_eq!(canonical.items()[0].deposit().unwrap().nonce, 46_388_024);
    for value in [None, Some(Value::Null)] {
        block["transactions"][0]
            .as_object_mut()
            .unwrap()
            .remove("nonce");
        if let Some(value) = value {
            block["transactions"][0]["nonce"] = value;
        }
        let missing = ordered(block.clone(), receipts.clone(), 84532).unwrap();
        assert_eq!(
            missing.items()[0].to_bytes(),
            canonical.items()[0].to_bytes()
        );
        assert_eq!(
            eth::simple_merkle_tree(&missing).root(),
            eth::simple_merkle_tree(&canonical).root()
        );
    }
}

#[test]
fn inconsistent_rpc_nonce_is_a_fallback_eligible_error() {
    let (mut block, receipts) = fixture(false);
    block["transactions"][0]["nonce"] = json!("0x0");
    let error = ordered(block, receipts, 84532).unwrap_err();
    assert!(matches!(
        error,
        Error::Deposit {
            source: DepositError::NonceMismatch { .. },
            ..
        }
    ));
    assert!(error.inconsistent_block_payload_for_fallback());
    assert_eq!(error.inconsistent_block_number_hint(), Some(46_388_021));
}

#[test]
fn changing_both_canyon_nonces_cannot_bypass_the_receipt_root() {
    let (mut block, mut receipts) = fixture(false);
    block["transactions"][0]["nonce"] = json!("0x0");
    receipts[0]["depositNonce"] = json!("0x0");
    assert!(matches!(
        ordered(block, receipts, 84532),
        Err(Error::BlockHeaderRootsMismatch(46_388_021))
    ));
}

#[test]
fn explicit_op_stack_never_uses_the_ethereum_pre_byzantium_receipt_exception() {
    let (mut block, mut receipts) = fixture(false);
    // A custom rollup can reuse a chain ID; Ethereum's historical exception must not
    // disable receipt validation for an explicitly configured OP-Stack chain.
    block["number"] = json!("0x1");
    block["transactions"][0]["nonce"] = json!("0x0");
    receipts[0]["depositNonce"] = json!("0x0");
    assert!(matches!(
        ordered(block, receipts, 1),
        Err(Error::BlockHeaderRootsMismatch(1))
    ));
}

#[test]
fn malformed_deposit_receipt_metadata_is_not_silently_discarded() {
    let (block, receipts) = fixture(false);
    for field in ["depositNonce", "depositReceiptVersion"] {
        for value in [json!("invalid"), json!("0x10000000000000000"), json!(true)] {
            let mut bad = receipts.clone();
            bad[0][field] = value;
            let error = ordered(block.clone(), bad, 84532).unwrap_err();
            assert!(
                matches!(
                    error,
                    Error::Deposit {
                        source: DepositError::ReceiptField { .. },
                        ..
                    }
                ),
                "{error}"
            );
        }
    }
}

#[test]
fn canyon_receipts_require_a_nonce_even_if_the_transaction_supplies_one() {
    let (block, mut receipts) = fixture(false);
    receipts[0].as_object_mut().unwrap().remove("depositNonce");
    assert!(matches!(
        ordered(block, receipts, 84532),
        Err(Error::Deposit {
            source: DepositError::ReceiptMissingNonce { .. },
            ..
        })
    ));
}

#[test]
fn pre_canyon_metadata_cannot_change_the_canonical_leaf() {
    let (block, receipts) = fixture(true);
    let canonical = ordered(block.clone(), receipts.clone(), 10).unwrap();
    assert_eq!(canonical.items()[0].deposit().unwrap().nonce, 0);
    let root = eth::simple_merkle_tree(&canonical).root();
    // Publicnode omits the transaction nonce for this real OP block. Other providers
    // supply it, but neither it nor pre-Canyon receipt nonce metadata is authenticated.
    for nonce in [None, Some(json!("0x0")), Some(json!("0xffff"))] {
        let mut changed_block = block.clone();
        let mut changed_receipts = receipts.clone();
        for object in [
            &mut changed_block["transactions"][0],
            &mut changed_receipts[0],
        ] {
            let key = if object.get("sourceHash").is_some() {
                "nonce"
            } else {
                "depositNonce"
            };
            object.as_object_mut().unwrap().remove(key);
            if let Some(value) = &nonce {
                object[key] = value.clone();
            }
        }
        let changed = ordered(changed_block, changed_receipts, 10).unwrap();
        assert_eq!(
            changed.items()[0].to_bytes(),
            canonical.items()[0].to_bytes()
        );
        assert_eq!(eth::simple_merkle_tree(&changed).root(), root);
    }
}

#[test]
fn historical_and_current_deposit_blocks_produce_valid_inclusion_proofs() {
    for historical in [false, true] {
        let (block, receipts) = fixture(historical);
        let block = ordered(block, receipts, if historical { 10 } else { 84532 }).unwrap();
        let tree = eth::simple_merkle_tree(&block);
        for (index, item) in block.items().iter().enumerate() {
            assert!(tree.generate_proof(index).unwrap().verify(&item.to_bytes()));
        }
    }
}

#[test]
fn rpc_deposit_gas_used_cannot_change_the_leaf_or_attestation_root() {
    for historical in [false, true] {
        let (block, receipts) = fixture(historical);
        let chain_id = if historical { 10 } else { 84532 };
        let canonical = ordered(block.clone(), receipts.clone(), chain_id).unwrap();
        let root = eth::simple_merkle_tree(&canonical).root();
        for gas_used in ["0x0", "0xdeadbeef", "0xffffffffffffffff"] {
            let mut changed_receipts = receipts.clone();
            changed_receipts[0]["gasUsed"] = json!(gas_used);
            let changed = ordered(block.clone(), changed_receipts, chain_id).unwrap();
            assert_eq!(changed.hash(), canonical.hash());
            assert_eq!(
                changed.items()[0].to_bytes(),
                canonical.items()[0].to_bytes()
            );
            assert_eq!(eth::simple_merkle_tree(&changed).root(), root);
        }
    }
}

/// Build a synthetic deposit-only block with consistent transaction/receipt roots so the
/// full pipeline exercises historical gas accounting, multiple deposits and RPC ordering.
/// The gas values supplied here are consensus receipt fields; RPC gasUsed is always bogus.
fn deposit_block(
    gas_limits: &[u64],
    cumulative_gas: &[u64],
    system: &[bool],
    canyon: bool,
) -> (Value, Value) {
    let (mut block, receipts) = fixture(false);
    let parsed_block: AnyRpcBlock = serde_json::from_value(block.clone()).unwrap();
    let parsed_receipts: Vec<AnyTransactionReceipt> =
        serde_json::from_value(receipts.clone()).unwrap();
    let original = DepositTransaction::try_from_rpc(
        parsed_block
            .transactions
            .as_transactions()
            .unwrap()
            .first()
            .unwrap(),
        DepositReceiptFields::from_other_fields(
            &parsed_receipts[0].other,
            parsed_receipts[0].transaction_hash,
        )
        .unwrap(),
    )
    .unwrap();
    let mut transactions = vec![];
    let mut new_receipts = vec![];
    let mut deposits = vec![];
    for (index, ((gas_limit, cumulative), is_system)) in gas_limits
        .iter()
        .zip(cumulative_gas)
        .zip(system)
        .enumerate()
    {
        let mut deposit = original.clone();
        deposit.source_hash = B256::repeat_byte(index as u8);
        deposit.gas_limit = *gas_limit;
        deposit.is_system_tx = *is_system;
        deposit.hash = keccak256(deposit.encoded_2718());
        let mut tx = block["transactions"][0].clone();
        tx["sourceHash"] = json!(deposit.source_hash);
        tx["gas"] = json!(format!("{gas_limit:#x}"));
        tx["isSystemTx"] = json!(is_system);
        tx["hash"] = json!(deposit.hash);
        tx["transactionIndex"] = json!(format!("{index:#x}"));
        let mut receipt = receipts[0].clone();
        receipt["transactionHash"] = json!(deposit.hash);
        receipt["transactionIndex"] = tx["transactionIndex"].clone();
        receipt["cumulativeGasUsed"] = json!(format!("{cumulative:#x}"));
        receipt["gasUsed"] = json!("0xdeadbeef");
        if !canyon {
            receipt.as_object_mut().unwrap().remove("depositNonce");
            receipt
                .as_object_mut()
                .unwrap()
                .remove("depositReceiptVersion");
        }
        transactions.push(tx);
        new_receipts.push(receipt);
        deposits.push(deposit);
    }
    let parsed_receipts: Vec<AnyTransactionReceipt> =
        serde_json::from_value(json!(new_receipts)).unwrap();
    block["transactionsRoot"] = json!(ordered_trie_root_with_encoder(&deposits, |tx, out| {
        tx.encode_2718(out);
    }));
    block["receiptsRoot"] = json!(ordered_trie_root_with_encoder(
        &parsed_receipts,
        |rx, out| {
            encode_deposit_receipt_2718(
                &rx.inner.inner,
                DepositReceiptFields::from_other_fields(&rx.other, rx.transaction_hash).unwrap(),
                rx.transaction_hash,
                out,
            )
            .unwrap();
        }
    ));
    transactions.reverse();
    new_receipts.reverse();
    block["transactions"] = json!(transactions);
    (block, json!(new_receipts))
}

fn deposit_gas_used(block: &OrderedBlock) -> Vec<u64> {
    block
        .items()
        .iter()
        .map(|item| match item {
            TxRx::OpDeposit { rx, .. } => rx.gas_used,
            _ => panic!("expected a deposit"),
        })
        .collect()
}

#[test]
fn pre_regolith_system_and_user_deposits_preserve_special_gas_accounting() {
    let (block, receipts) =
        deposit_block(&[150_000_000, 90_000], &[0, 90_000], &[true, false], false);
    let block = ordered(block, receipts, 84532).unwrap();
    assert_eq!(deposit_gas_used(&block), vec![0, 90_000]);
}

#[test]
fn regolith_gas_uses_receipt_deltas_before_and_after_canyon() {
    for canyon in [false, true] {
        let (block, receipts) = deposit_block(
            &[1_000_000, 400_000],
            &[120_000, 200_000],
            &[false, false],
            canyon,
        );
        let block = ordered(block, receipts, 84532).unwrap();
        assert_eq!(deposit_gas_used(&block), vec![120_000, 80_000]);
    }
}

#[test]
fn decreasing_cumulative_deposit_gas_is_a_fallback_eligible_error() {
    let (block, receipts) = deposit_block(
        &[1_000_000, 400_000],
        &[120_000, 100_000],
        &[false, false],
        true,
    );
    let error = ordered(block, receipts, 84532).unwrap_err();
    assert!(matches!(
        error,
        Error::Deposit {
            source: DepositError::DecreasingCumulativeGas {
                previous: 120_000,
                cumulative: 100_000,
                ..
            },
            ..
        }
    ));
    assert!(error.inconsistent_block_payload_for_fallback());
    assert_eq!(error.inconsistent_block_number_hint(), Some(46_388_021));
}
