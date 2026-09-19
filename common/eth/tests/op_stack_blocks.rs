use alloy::network::{AnyRpcBlock, AnyTransactionReceipt};
use eth::{op_stack::DepositError, ChainFamily, Error, OrderedBlock};
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
