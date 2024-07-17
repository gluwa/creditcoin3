use utils::block_item_traits::FetchFromBlock;
use eth_common::transaction::{BlockItem, Receipt, Transaction};
//  use common::sorted_block::SortedBlockError;
use utils::{Felt, StarknetPedersenMmr};
use mmr::traits::MerkleTreeTrait;
use ethereum_types::U256;
use eth_common::{fetch_block_transactions, fetch_block_receipts};

pub async fn retrieve_and_compute_merkle_trees(
    url: &str,
//    cache_dir: Option<&str>,
    block_number: U256,
) -> anyhow::Result<(StarknetPedersenMmr, StarknetPedersenMmr)> {
//) -> Result<(StarknetPedersenMmr, StarknetPedersenMmr), SortedBlockError> {
    // let mut tx_cache =
    //     cache_dir.map(|dir| <TypedTransaction as FetchFromBlock>::Cache::new(dir, block_number));
    // let mut rx_cache =
    //     cache_dir.map(|dir| <Receipt as FetchFromBlock>::Cache::new(dir, block_number));

    let tx_merkle_tree_fut = fetch_block_transactions(url, block_number.as_u64());
    let rx_merkle_tree_fut = fetch_block_receipts(url, block_number.as_u64());

    let (txs, rxs) = futures::future::try_join(tx_merkle_tree_fut, rx_merkle_tree_fut).await?;

    let txs_bytes = txs
        .into_iter()
        .map(|tx| tx.to_bytes())
        .collect::<Vec<_>>();
    let rxs_bytes = rxs
        .into_iter()
        .map(|rx| rx.to_bytes())
        .collect::<Vec<_>>();

    Ok((StarknetPedersenMmr::from(&txs_bytes[..]), StarknetPedersenMmr::from(&rxs_bytes[..])))
}

pub async fn retrieve_and_compute_merkle_roots(
    url: &str,
//    cache_dir: Option<&str>,
    block_number: U256,
) -> anyhow::Result<(Felt, Felt)> {
//) -> Result<(Felt, Felt), SortedBlockError> {
    let (tx_tree, rx_tree) =
        retrieve_and_compute_merkle_trees(url, block_number).await?;

    Ok((tx_tree.root().0, rx_tree.root().0))
}
