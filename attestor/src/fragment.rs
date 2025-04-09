use anyhow::Result;

use attestation_chain::continuity_chain::{CreateResult, Manager};
use eth::Client;
use sp_core::H256;

pub async fn async_retry_create(
    end_block: u64,
    attestation_interval: u64,
    eth_client: &Client,
    prev_digest: H256,
) -> Result<CreateResult> {
    let start_block = end_block.saturating_sub(attestation_interval) + 1;
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
