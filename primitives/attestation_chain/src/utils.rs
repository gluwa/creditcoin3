use eth_common::{transaction::BlockItem, Client};
use ethereum_types::U256;
use mmr::traits::MerkleTreeTrait;
use utils::{Felt, StarknetPedersenMmr};

pub async fn retrieve_and_compute_merkle_trees(
    url: &str,
    block_number: U256,
) -> anyhow::Result<(StarknetPedersenMmr, StarknetPedersenMmr)> {
    let eth_client = Client::new(url).await?;

    let tx_merkle_tree_fut = eth_client.get_transactions(block_number.as_u64());
    let rx_merkle_tree_fut = eth_client.get_receipts(block_number.as_u64());

    let (txs, rxs) = futures::future::try_join(tx_merkle_tree_fut, rx_merkle_tree_fut).await?;

    let txs_bytes = txs.into_iter().map(|tx| tx.to_bytes()).collect::<Vec<_>>();
    let rxs_bytes = rxs.into_iter().map(|rx| rx.to_bytes()).collect::<Vec<_>>();

    Ok((
        StarknetPedersenMmr::from(&txs_bytes[..]),
        StarknetPedersenMmr::from(&rxs_bytes[..]),
    ))
}

pub async fn retrieve_and_compute_merkle_roots(
    url: &str,
    block_number: U256,
) -> anyhow::Result<(Felt, Felt)> {
    let (tx_tree, rx_tree) = retrieve_and_compute_merkle_trees(url, block_number).await?;

    Ok((tx_tree.root().0, rx_tree.root().0))
}
