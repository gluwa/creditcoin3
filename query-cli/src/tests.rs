use super::Network;
use crate::query_builder::BlockscoutAbiProvider;
use usc_query_builder::abi::query_builder::AbiProvider;

#[test]
fn source_family_flag_is_available_for_every_query_mode() {
    use clap::Parser;
    let common = [
        "query-cli",
        "--cc3-rpc-url",
        "http://localhost:9944",
        "--cc3-evm-private-key",
        "test-key",
    ];
    for command in [
        vec!["verify"],
        vec![
            "transfer",
            "--eth-rpc-url",
            "http://custom",
            "--eth-private-key",
            "test-key",
            "--to-address",
            "0x0000000000000000000000000000000000000001",
            "--amount-wei",
            "1",
            "--chain-key",
            "20",
        ],
        vec![
            "batch-transfer",
            "--eth-rpc-url",
            "http://custom",
            "--eth-private-key",
            "test-key",
            "--chain-key",
            "20",
        ],
    ] {
        let mut args = common.to_vec();
        args.extend(command);
        let omitted = super::QueryCli::try_parse_from(args.clone()).unwrap();
        assert_eq!(omitted.eth_chain_family, None);
        args.extend(["--eth-chain-family", "ethereum"]);
        let explicit = super::QueryCli::try_parse_from(args.clone()).unwrap();
        assert_eq!(explicit.eth_chain_family, Some(eth::ChainFamily::Ethereum));
        *args.last_mut().unwrap() = "op-stack";
        let parsed = super::QueryCli::try_parse_from(args.clone()).unwrap();
        assert_eq!(parsed.eth_chain_family, Some(eth::ChainFamily::OpStack));
        *args.last_mut().unwrap() = "unsupported";
        assert!(super::QueryCli::try_parse_from(args).is_err());
    }
}

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
