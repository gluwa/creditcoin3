use crate::AsyncCallbackWithArg;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use utils::Felt;

#[derive(Debug, Clone)]
pub enum CreateAttestationBlockError {
    Network(String),
    ResiliencyEventLoopDropped(u64),
    //    Network(jsonrpsee_core::ClientError),
    Cancelled(u64),
    Other(String),
}

pub(crate) async fn create_attestation_block_task(
    source_chain_api_server_url: Arc<str>,
    cache_dir: Option<Arc<str>>,
    block_number: u64,
    create_attestation_block_cancellation_token: CancellationToken,
    disconnected: Arc<AtomicBool>,
    retrial_period: u64,
    retrial_attempts: usize,

    on_retry_retrieve_block: Option<AsyncCallbackWithArg<(u64, String, u64), ()>>,
    on_toggle_connection_mode: Option<AsyncCallbackWithArg<bool, ()>>,
) -> Result<Felt, CreateAttestationBlockError> {
    let mut retrials = 0;

    loop {
        match disconnected.load(AtomicOrdering::Relaxed) {
            false => {
                let create_attestation_block_cancellation_token =
                    create_attestation_block_cancellation_token.clone();
                // download transactions and receipts
                // compute Pedersen hashes and create attestation block
                match retrieve_block_and_compute_merkle_roots(
                    &source_chain_api_server_url,
                    cache_dir.as_deref(),
                    block_number,
                    create_attestation_block_cancellation_token,
                )
                .await
                {
                    Ok(root) => break Ok(root),

                    Err(CreateAttestationBlockError::Network(msg)) => {
                        if let Some(ref callback) = on_retry_retrieve_block {
                            callback((block_number, msg, retrial_period)).await;
                        }

                        sleep(Duration::from_millis(retrial_period)).await;
                        retrials += 1;
                        if retrials >= retrial_attempts {
                            if let Some(ref callback) = on_toggle_connection_mode {
                                callback(!disconnected.load(AtomicOrdering::Relaxed)).await;
                            }
                            disconnected.store(true, AtomicOrdering::Relaxed);
                        }
                    }
                    Err(err) => break Err(err), // other non-recoverable errors
                }
            }

            true => {
                sleep(Duration::from_millis(retrial_period)).await;

                if create_attestation_block_cancellation_token.is_cancelled() {
                    break Err(CreateAttestationBlockError::Cancelled(block_number));
                }
            }
        }
    }
}

async fn retrieve_block_and_compute_merkle_roots(
    url: &str,
    cache_dir: Option<&str>,
    block_number: u64,
    cancellation_token: CancellationToken,
) -> Result<Felt, CreateAttestationBlockError> {

    tokio::select! {
        res = retrieve_block_and_compute_merkle_root_cached(url, cache_dir, block_number) => {
            match res {
                Ok(tree_root) => Ok(tree_root),
                Err(err) => Err(CreateAttestationBlockError::Network(err.to_string())),
            }
        }

        _ = cancellation_token.cancelled() => {
            Err(CreateAttestationBlockError::Cancelled(block_number))
        },
    }
}

async fn retrieve_block_and_compute_merkle_root_cached(
    url: &str,
    cache_dir: Option<&str>,
    block_number: u64,
) -> anyhow::Result<Felt> {
    use block_cache::BlockCache;
    use block_cache::CacheT;
    use mmr::traits::MerkleTreeTrait;
    use block_cache::OrderedBlockJson;

    let mut cache =
        cache_dir.map(|dir| BlockCache::new(dir, block_number));

    if let Some(ref cache) = cache {
        if let Ok(block_json) = cache.try_read() {
            let block = eth_common::OrderedBlock::try_create(
                block_json.chain_id.unwrap(),
                block_json.number,
                block_json.hash,
                block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
                block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
            )?;

            return Ok(eth_common::starknet_pedersen_mmr(&block).root().0);
        }
    }

    let eth_client = eth_common::Client::new(url, "").await?;
    let raw_block = eth_client.get_raw_block(block_number).await?;

    let block_json = OrderedBlockJson {
        chain_id: raw_block.chain_id,
        number: raw_block.number,
        hash: raw_block.hash,
        items: raw_block.transactions.into_iter().zip(raw_block.receipts.into_iter()).collect(),
    };

    if let Some(cache) = cache.as_mut() {
        let _ = cache.try_write(&block_json);
    }

    let block = eth_common::OrderedBlock::try_create(
        block_json.chain_id.unwrap(),
        block_json.number,
        block_json.hash,
        block_json.items.iter().map(|(tx, _)| tx).cloned().collect(),
        block_json.items.iter().map(|(_, rx)| rx).cloned().collect(),
    )?;

    Ok(eth_common::starknet_pedersen_mmr(&block).root().0)
}
