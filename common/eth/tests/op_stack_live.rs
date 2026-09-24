//! Optional, read-only source RPC smoke test. See docs/op-stack-e2e.md.
use eth::{ChainFamily, Client};
use usc_abi_encoding::common::EncodingVersion;
use utils::block_item_traits::BlockItem;

#[tokio::test]
#[ignore = "requires OP_STACK_RPC_URL and access to a live source RPC"]
async fn live_block_roots_and_all_transaction_proofs_verify() {
    let url = std::env::var("OP_STACK_RPC_URL").expect("set OP_STACK_RPC_URL");
    let client = Client::new(&url, None)
        .await
        .expect("connect source RPC")
        .with_chain_family_override(Some(ChainFamily::OpStack));
    let height = match std::env::var("OP_STACK_BLOCK_NUMBER") {
        Ok(height) => height.parse().expect("decimal block number"),
        Err(_) => client.get_last_block().await.unwrap().saturating_sub(1_000),
    };
    let block = client
        .get_block(height, EncodingVersion::V1)
        .await
        .expect("verify both header roots");
    assert!(
        block.items().iter().any(|tx| tx.deposit().is_some()),
        "choose a block containing deposits"
    );
    let tree = eth::simple_merkle_tree(&block);
    for (index, tx) in block.items().iter().enumerate() {
        assert!(tree.generate_proof(index).unwrap().verify(&tx.to_bytes()));
    }
    println!(
        "chain={} block={} transactions={} root={:?}",
        client.chain_id(),
        height,
        block.items().len(),
        tree.root()
    );
    if url.starts_with("ws://") || url.starts_with("wss://") {
        use futures_util::StreamExt;
        let mut headers = client.subscribe().await.expect("subscribe to source heads");
        let header = tokio::time::timeout(std::time::Duration::from_secs(30), headers.next())
            .await
            .expect("receive a new head within 30 seconds")
            .expect("subscription remains open");
        println!("subscription received block {}", header.number);
    }
}
