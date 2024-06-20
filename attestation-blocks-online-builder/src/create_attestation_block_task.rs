#![allow(clippy::too_many_arguments)]

use crate::AsyncCallbackWithArg;
//use common::sorted_block::SortedBlockError;
use ethereum_types::U256;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tokio_util::sync::CancellationToken;
use utils::Felt;

#[derive(Debug, Clone)]
pub enum CreateAttestationBlockError {
    Network(String),
    ResiliencyEventLoopDropped(U256),
    //    Network(jsonrpsee_core::ClientError),
    //    Empty(U256),
    Cancelled(U256),
    Other(String),
}

// impl From<SortedBlockError> for CreateAttestationBlockError {
//     fn from(err: SortedBlockError) -> Self {
//         match err {
//             SortedBlockError::FetchFailure(err) => Self::Network(format!("{err:?}")),
//             err => unreachable!("unexpected error: {err:?}"),
//         }
//     }
// }

pub(crate) async fn create_attestation_block_task(
    source_chain_api_server_url: Arc<str>,
    _cache_dir: Option<Arc<str>>,
    block_number: U256,
    create_attestation_block_cancellation_token: CancellationToken,
    disconnected: Arc<AtomicBool>,
    retrial_period: u64,
    retrial_attempts: usize,

    on_retry_retrieve_block: Option<AsyncCallbackWithArg<(U256, String, u64), ()>>,
    on_toggle_connection_mode: Option<AsyncCallbackWithArg<bool, ()>>,
) -> Result<(Felt, Felt), CreateAttestationBlockError> {
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
                    //                    cache_dir.as_deref(),
                    block_number,
                    create_attestation_block_cancellation_token,
                )
                .await
                {
                    Ok(roots) => break Ok(roots),

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
    //    cache_dir: Option<&str>,
    block_number: U256,
    cancellation_token: CancellationToken,
) -> Result<(Felt, Felt), CreateAttestationBlockError> {
    use attestation_chain::utils::retrieve_and_compute_merkle_roots;

    tokio::select! {
            res = retrieve_and_compute_merkle_roots(url, block_number) => {
    //            res = retrieve_and_compute_merkle_roots(url, cache_dir, block_number) => {
                match res {
                    Ok((tx_tree_root, rx_tree_root)) => Ok((tx_tree_root, rx_tree_root)),
                    Err(err) => Err(CreateAttestationBlockError::Network(err.to_string())),
    //                Err(err) => Err(CreateAttestationBlockError::from(err)),
                }
            }

            _ = cancellation_token.cancelled() => {
                Err(CreateAttestationBlockError::Cancelled(block_number))
            },
        }
}
