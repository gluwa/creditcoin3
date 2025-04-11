use anyhow::Result;

pub use attestation_chain::continuity_chain::CreateResult;
use attestation_chain::continuity_chain::Manager;
use eth::Client;
use sp_core::H256;

pub async fn async_retry_create(
    eth_client: &Client,
    end_block: u64,
    fragment_length: u64,
    prev_digest: H256,
) -> Result<CreateResult> {
    let start_block = end_block.saturating_sub(fragment_length) + 1;
    let fragment_manager = Manager::new(start_block, end_block, eth_client);

    let fragment: CreateResult = crate::retry::ret(
        || async { fragment_manager.create(prev_digest).await },
        10,
        10,
        Some(60),
    )
    .await?;

    Ok(fragment)
}
