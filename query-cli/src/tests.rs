use super::Network;
use crate::query_builder::BlockscoutAbiProvider;
use usc_query_builder::abi::query_builder::AbiProvider;

#[tokio::test]
// Ignoring this test in CI because it depends on Blockscout stability, which is flaky
#[ignore]
async fn test_blockscout_sepolia_abi_retrieval() {
    // GCRE contract address
    let address = String::from("0x47C30768E4c153B40d55b90F58472bb2291971e6");

    let abi_provider = BlockscoutAbiProvider {
        network: Network::Sepolia(String::from("dummy_api_key")),
    };
    let result = abi_provider.get_abi(address).await;
    match result {
        Ok(abi) => println!("ABI: {abi:?}"),
        Err(error) => panic!("Error: {error:?}"),
    }
}

#[tokio::test]
// Ignoring this test in CI because it depends on Blockscout stability, which is flaky
#[ignore]
async fn test_blockscout_eth_abi_retrieval() {
    // Uniswap V2 pair contract
    let address = String::from("0x8aAf4585FA29276cBb5ab17216473d064784b527");

    let abi_provider = BlockscoutAbiProvider {
        network: Network::Ethereum(String::new()),
    };
    let result = abi_provider.get_abi(address).await;
    match result {
        Ok(abi) => println!("ABI: {abi:?}"),
        Err(error) => panic!("Error: {error:?}"),
    }
}
