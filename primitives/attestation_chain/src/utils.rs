use utils::block_item_traits::FetchFromBlock;
use eth_common::transaction::{Transaction, Receipt};
//  use common::sorted_block::SortedBlockError;
use utils::{Felt, StarknetPedersenMmr};
use mmr::traits::MerkleTreeTrait;
use ethereum_types::U256;

// pub async fn retrieve_and_compute_merkle_trees(
//     url: &str,
//     cache_dir: Option<&str>,
//     block_number: U256,
// ) -> Result<(StarknetPedersenMmr, StarknetPedersenMmr), SortedBlockError> {
//     let mut tx_cache =
//         cache_dir.map(|dir| <TypedTransaction as FetchFromBlock>::Cache::new(dir, block_number));
//     let mut rx_cache =
//         cache_dir.map(|dir| <Receipt as FetchFromBlock>::Cache::new(dir, block_number));

//     let tx_merkle_tree_fut =
//         common::build_tx_or_rx_mmr::<TypedTransaction>(url, tx_cache.as_mut(), block_number);
//     let rx_merkle_tree_fut =
//         common::build_tx_or_rx_mmr::<Receipt>(url, rx_cache.as_mut(), block_number);

//     futures::future::try_join(tx_merkle_tree_fut, rx_merkle_tree_fut).await
// }

// pub async fn retrieve_and_compute_merkle_roots(
//     url: &str,
//     cache_dir: Option<&str>,
//     block_number: U256,
// ) -> Result<(Felt, Felt), SortedBlockError> {
//     let (tx_tree, rx_tree) =
//         retrieve_and_compute_merkle_trees(url, cache_dir, block_number).await?;

//     Ok((tx_tree.root().0, rx_tree.root().0))
// }
