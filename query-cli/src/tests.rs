use super::Network;
use crate::query_builder::{
    BlockscoutAbiProvider, get_erc20_transfer_segments,
};
use ccnext_query_builder::{
    abi::query_builder::AbiProvider,
    test_helpers::{
        get_transaction_and_receipt, ResultField, check_results,
    },
};
use ccnext_abi_encoding::abi::abi_encode;
use alloy::consensus::Transaction;

#[tokio::test]
async fn test_blockscout_sepolia_abi_retrieval() {
    // GCRE contract address
    let address = String::from("0x47C30768E4c153B40d55b90F58472bb2291971e6");

    let abi_provider = BlockscoutAbiProvider {
        network: Network::Sepolia,
    };
    let result = abi_provider.get_abi(address).await;
    match result {
        Ok(abi) => println!("ABI: {:?}", abi),
        Err(error) => panic!("Error: {:?}", error),
    }
}

#[tokio::test]
async fn test_blockscout_eth_abi_retrieval() {
    // Uniswap V2 pair contract
    let address = String::from("0x8aAf4585FA29276cBb5ab17216473d064784b527");

    let abi_provider = BlockscoutAbiProvider {
        network: Network::Ethereum,
    };
    let result = abi_provider.get_abi(address).await;
    match result {
        Ok(abi) => println!("ABI: {:?}", abi),
        Err(error) => panic!("Error: {:?}", error),
    }
}

#[tokio::test]
async fn test_get_erc20_transfer_segments_with_sepolia_gcre() {
    // Get legacy transaction via rpc
    let (tx, rx) = get_transaction_and_receipt(
        "0xc990ce703dd3ca83429c302118f197651678de359c271f205b9083d4aa333aae",
    )
    .await;
    assert!(tx.inner.is_legacy());

    // Encode transaction
    let raw = abi_encode(tx.clone(), rx.clone()).unwrap().abi;

    let selected_offsets: Vec<(usize, usize)> = get_erc20_transfer_segments(Network::Sepolia, tx.clone(), rx.clone()).await.expect("Getting segments should succeed").iter().map(|segment| {
        (segment.offset as usize, segment.size as usize)
    }).collect();

    let expected_results: Vec<ResultField> = vec![
        ResultField::RxStatus(rx.status() as u8),
        ResultField::EthAddress(tx.from), // Caller address
        ResultField::EthAddress(tx.to().expect("Should be to field in contract call")), // Contract address in call
        ResultField::EthAddress(rx.inner.logs()[1].address()), // Contract address in event
        ResultField::EventTopic(rx.inner.logs()[1].topic0().unwrap().0), // Event signature
        ResultField::EventTopic(rx.inner.logs()[1].topics()[1].0), // Event from address
        ResultField::EventTopic(rx.inner.logs()[1].topics()[2].0), // Event to address
        ResultField::EventDataField(
            rx.inner.logs()[1].data().data[..]
                .try_into()
                .expect("Data should contain 1 32 byte field for this transaction"),
        ), // Event value
    ];

    // Checking that all result data matches expected
    check_results(expected_results, selected_offsets, raw.clone());
}