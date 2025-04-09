use eth::Client;
use mmr::traits::MerkleTreeTrait;
use utils::{Felt, StarknetPedersenMerkleTree};

pub async fn retrieve_and_compute_merkle_tree(
    url: &str,
    block_number: u64,
) -> anyhow::Result<StarknetPedersenMerkleTree> {
    let eth_client = Client::new(url, None).await?;
    let block = eth_client.get_block(block_number).await?;

    Ok(eth::starknet_pedersen_mmr(&block))
}

pub async fn retrieve_and_compute_merkle_root(
    url: &str,
    block_number: u64,
) -> anyhow::Result<Felt> {
    retrieve_and_compute_merkle_tree(url, block_number)
        .await
        .map(|mt| mt.root().0)
}
