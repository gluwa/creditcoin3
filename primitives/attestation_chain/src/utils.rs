use eth_common::Client;
use mmr::traits::MerkleTreeTrait;
use utils::{Felt, StarknetPedersenMerkleTree};

pub async fn retrieve_and_compute_merkle_trees(
    url: &str,
    block_number: u64,
) -> anyhow::Result<StarknetPedersenMerkleTree> {
    let eth_client = Client::new(url, "").await?;
    let block = eth_client.get_block(block_number).await?;

    Ok(eth_common::starknet_pedersen_mmr(&block))
    // let tx_merkle_tree_fut = eth_client.get_transactions(block_number);
    // let rx_merkle_tree_fut = eth_client.get_receipts(block_number);

    // let (txs, rxs) = futures::future::try_join(tx_merkle_tree_fut, rx_merkle_tree_fut).await?;

    // let txs_bytes = txs.into_iter().map(|tx| tx.to_bytes()).collect::<Vec<_>>();
    // let rxs_bytes = rxs.into_iter().map(|rx| rx.to_bytes()).collect::<Vec<_>>();

    // Ok((
    //     StarknetPedersenMerkleTree::from(&txs_bytes[..]),
    //     StarknetPedersenMerkleTree::from(&rxs_bytes[..]),
    // ))
}

pub async fn retrieve_and_compute_merkle_root(
    url: &str,
    block_number: u64,
) -> anyhow::Result<Felt> {
    retrieve_and_compute_merkle_trees(url, block_number)
        .await
        .map(|mt| mt.root().0)
}
