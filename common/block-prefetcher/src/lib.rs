use alloy::transports::{RpcError, TransportErrorKind};
use thiserror::Error;
use tokio_retry::strategy::{jitter, ExponentialBackoff};
use tokio_retry::Retry;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Error)]
pub enum PrefetchError {
    #[error("Block not found at height {0}")]
    BlockNotFound(u64),
    #[error("Receipts not found at height {0}")]
    ReceiptsNotFound(u64),
    #[error("Transaction/receipt count mismatch at height {height}: {tx_count} transactions vs {receipt_count} receipts")]
    TransactionReceiptMismatch {
        height: u64,
        tx_count: usize,
        receipt_count: usize,
    },
    #[error("Failed to create ordered block at height {height}: {reason}")]
    OrderedBlockCreation { height: u64, reason: String },
    #[error("RPC transport error: {0}")]
    Transport(#[from] RpcError<TransportErrorKind>),
}

/// Configuration for the block prefetcher.
pub struct PrefetchConfig {
    /// Chain ID to use when creating ordered blocks (default: 1 for Ethereum mainnet)
    pub chain_id: u64,
    /// Maximum number of concurrent block fetches (default: 10)
    pub max_concurrent_fetches: usize,
    /// Number of blocks to lag behind the source chain to avoid reorgs (default: 10 blocks)
    pub finalization_lag: u64,
    /// Maximum number of retry attempts for RPC errors (default: 5)
    pub max_retries: usize,
    /// Initial delay between retries in milliseconds (default: 200)
    pub retry_base_delay_ms: u64,
}

impl Default for PrefetchConfig {
    fn default() -> Self {
        Self {
            chain_id: 1,
            max_concurrent_fetches: 10,
            finalization_lag: 10,
            max_retries: 5,
            retry_base_delay_ms: 200,
        }
    }
}

/// Block prefetcher that continuously fetches blocks from a provider and pushes them to a sink.
/// The prefetcher handles retries with exponential backoff and ensures that blocks are fetched in order.
/// The prefetcher will lag behind the tip of the chain by a configurable number of blocks to avoid reorgs.
///
/// The prefetcher is designed to be resilient to RPC errors and will retry failed fetches up to a configurable maximum number of attempts.
///
/// The prefetcher can be gracefully stopped by requesting cancellation, which will cause the main loop to exit after completing any in-flight fetches.
pub struct BlockPrefetcher<P, S> {
    config: PrefetchConfig,
    provider: P,
    sink: S,
    cancellation_token: CancellationToken,
}

impl<P, S> BlockPrefetcher<P, S>
where
    P: alloy::providers::Provider + Send + Sync,
    S: block_primitives::BlockSink<Block = eth::OrderedBlock> + Send,
{
    pub fn new(config: PrefetchConfig, provider: P, sink: S) -> Self {
        Self {
            config,
            provider,
            sink,
            cancellation_token: CancellationToken::new(),
        }
    }

    /// Returns a handle to request cancellation
    #[must_use]
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancellation_token.clone()
    }

    /// Fetch a single block at the given height with retry logic.
    ///
    /// Returns an ordered block containing transactions and receipts.
    async fn try_fetch_block(&self, height: u64) -> Result<eth::OrderedBlock, PrefetchError> {
        tracing::debug!(height, "Attempting to fetch block");

        let retry_strategy = ExponentialBackoff::from_millis(self.config.retry_base_delay_ms)
            .map(jitter)
            .take(self.config.max_retries);

        let action = || async move {
            let (block, receipts) = {
                let block_fut = self.provider.get_block(height.into(), true.into());
                let receipts_fut = self.provider.get_block_receipts(height.into());

                match futures::future::try_join(block_fut, receipts_fut).await {
                    Ok((Some(block), Some(receipts))) => Ok((block, receipts)),
                    Ok((None, _)) => Err(PrefetchError::BlockNotFound(height)),
                    Ok((_, None)) => Err(PrefetchError::ReceiptsNotFound(height)),
                    Err(err) => {
                        tracing::debug!(
                            height,
                            error = err.to_string(),
                            "Failed to fetch block, retrying..."
                        );
                        Err(PrefetchError::Transport(err))
                    }
                }?
            };

            let tx_count = block.transactions.len();
            let receipt_count = receipts.len();
            if tx_count != receipt_count {
                return Err(PrefetchError::TransactionReceiptMismatch {
                    height,
                    tx_count,
                    receipt_count,
                });
            }

            eth::OrderedBlock::try_create(
                self.config.chain_id,
                height,
                block.header.hash,
                block.transactions.into_transactions_vec(),
                receipts,
                ccnext_abi_encoding::common::EncodingVersion::V1,
            )
            .map_err(|e| PrefetchError::OrderedBlockCreation {
                height,
                reason: e.to_string(),
            })
        };

        Retry::spawn(retry_strategy, action).await
    }

    /// Start the prefetcher loop that continuously fetches new blocks.
    ///
    /// # Cancellation
    ///
    /// The loop can be stopped by calling `cancel()` on the token returned by
    /// [`cancel_token()`](Self::cancel_token). Note that blocks fetched but not
    /// yet pushed to the sink may be lost on cancellation.
    ///
    /// # Errors
    ///
    /// Returns an error if an unrecoverable RPC error occurs or if resubscription
    /// fails after exhausting retries.
    pub async fn run(mut self) -> Result<(), PrefetchError> {
        use futures::stream::StreamExt as _;
        use futures::stream::TryStreamExt as _;

        let mut tip = self.provider.get_block_number().await?;
        let mut subscription = self.provider.subscribe_blocks().await?.into_stream();

        tracing::info!(tip, "Starting block prefetcher loop");

        loop {
            // Determine range of blocks to fetch based on sink state and current tip
            let fetch_from = self.sink.next_needed_height();
            let fetch_to = match tip.checked_sub(self.config.finalization_lag) {
                Some(t) => t,
                None => {
                    tracing::debug!(
                        tip,
                        finalization_lag = self.config.finalization_lag,
                        "Tip is below finalization lag, waiting for more blocks..."
                    );

                    // Wait for a short period before checking again to avoid busy looping
                    // when fetching from the start of the chain and the chain is producing slowly
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;

                    // Update tip before next loop iteration to check if we can start fetching
                    tip = self.provider.get_block_number().await?;

                    continue;
                }
            };

            tracing::debug!(fetch_from, fetch_to, tip, "Determined block range to fetch");

            // Only fetch if we have new finalized blocks to process
            if fetch_from <= fetch_to {
                // Fetch blocks in parallel with retry logic
                let blocks = futures::stream::iter(fetch_from..=fetch_to)
                    .map(|height| self.try_fetch_block(height))
                    .buffered(self.config.max_concurrent_fetches)
                    .try_collect::<Vec<_>>()
                    .await?;

                self.sink.push(blocks);
            }

            // Wait for next header with cancellation support
            tokio::select! {
                biased;

                _ = self.cancellation_token.cancelled() => {
                    tracing::info!("Cancellation requested, exiting loop");
                    break Ok(());
                }
                header = subscription.next() => {
                    match header {
                        Some(header) => {
                            tip = header.number;
                            tracing::debug!(tip, "Received new block header, advancing tip");
                        }
                        None => {
                            tracing::warn!("Block subscription stream ended, trying to resubscribe...");

                            // We use the same retry strategy for resubscription to handle transient issues
                            let retry_strategy = ExponentialBackoff::from_millis(self.config.retry_base_delay_ms)
                            .map(jitter)
                            .take(self.config.max_retries);

                            let action = || async {
                                self.provider.subscribe_blocks().await
                                    .map(|sub| sub.into_stream())
                                    .map_err(|err| {
                                        tracing::debug!(
                                            error = err.to_string(),
                                            "Failed to subscribe to blocks, retrying..."
                                        );
                                        PrefetchError::Transport(err)
                                    })
                            };

                            match Retry::spawn(retry_strategy, action).await {
                                Ok(new_sub) => {
                                    subscription = new_sub;
                                    tracing::info!("Successfully resubscribed to block headers");
                                }
                                Err(err) => {
                                    tracing::error!(
                                        error = err.to_string(),
                                        "Failed to resubscribe to blocks after retries, exiting loop"
                                    );
                                    break Err(err);
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
