use ccnext_abi_encoding::abi::EncodingVersion;
use eth::{Client as EthClient, OrderedBlock};
use pallet_prover_primitives::Query;
use thiserror::Error;
use tokio_retry::strategy::{jitter, FibonacciBackoff};
use tokio_retry::Retry;
use utils::block_item_traits::BlockItem;

// Query defects
// - Query with 0 layout segments
// - Block doesn’t exist yet
// - Tx number not contained in block
// - Selected data out of range contained within Selected tx

#[derive(Debug, Error)]
pub enum Error {
    #[error("A query must have at least one byte of data to prove.")]
    EmptyQuery,
    #[error("The TxRx item for the given block and transaction has no data.")]
    EmptyTxRx,
    #[error(
        "Query corresponds to non-existant block. Query height: {0}, Highest source block: {1}"
    )]
    NoSuchBlock(u64, u64),
    #[error("No such tx in block. Query tx index: {0}, Max index in block: {1}")]
    NoSuchTxInBlock(usize, usize),
    #[error("No such data in tx. Segment: {segment}, Queried index: {end}, Max index contained in TxRx: {payload_len}")]
    NoSuchDataInTxRx {
        segment: usize,
        end: usize,
        payload_len: usize,
    },
    #[error("Layout segment offset + size result in integer overflow. Segment idx: {segment}")]
    OffsetOverflow { segment: usize },
    #[error("Missing tx even after checking for its existance. This shouldn't happen.")]
    MissingTx,
    #[error(transparent)]
    EthError(#[from] eth::Error),
}

impl Error {
    pub fn is_permanently_invalid(&self) -> bool {
        matches!(
            self,
            Error::EmptyQuery
                | Error::EmptyTxRx
                | Error::NoSuchBlock(_, _)
                | Error::NoSuchTxInBlock(_, _)
                | Error::NoSuchDataInTxRx { .. }
                | Error::OffsetOverflow { .. }
        )
    }
}

/// Checks to make sure that a query isn't obviously invalid before processing it.
pub(crate) async fn pre_check_query(
    query: &Query,
    eth_client: &EthClient,
    encoding: EncodingVersion,
) -> Result<(), Error> {
    check_highest_source_block(query, eth_client).await?;
    let query_block = get_query_block(query.height, eth_client, encoding).await?;
    check_tx_exists_in_block(query, &query_block)?;
    check_queried_bytes_against_txrx(query, &query_block)?;
    Ok(())
}

/// Checks that the most recent source chain block is at a height >= the query height.
async fn check_highest_source_block(query: &Query, eth_client: &EthClient) -> Result<(), Error> {
    // Retry strategy with Fibonacci backoff and jitter (1, 1, 2, 3, 5, ...)
    let retry_strategy = FibonacciBackoff::from_millis(1000).map(jitter).take(5);
    let highest_block = Retry::spawn(retry_strategy, || eth_client.get_last_block()).await?;
    if highest_block >= query.height {
        Ok(())
    } else {
        Err(Error::NoSuchBlock(query.height, highest_block))
    }
}

/// Gets the query block, with retries
async fn get_query_block(
    block_height: u64,
    eth_client: &EthClient,
    encoding: EncodingVersion,
) -> Result<OrderedBlock, Error> {
    // Retry strategy with Fibonacci backoff and jitter (1, 1, 2, 3, 5, ...)
    let retry_strategy = FibonacciBackoff::from_millis(1000).map(jitter).take(5);
    let block = Retry::spawn(retry_strategy, || {
        eth_client.get_block(block_height, encoding)
    })
    .await?;
    Ok(block)
}

/// Checks that there is a tx at the queried index within the queried block
fn check_tx_exists_in_block(query: &Query, block: &OrderedBlock) -> Result<(), Error> {
    if block.items().is_empty() {
        return Err(Error::NoSuchTxInBlock(query.index as usize, 0));
    }
    let last_index = block.items().len() - 1; // Made safe by the is_empty() check immediately above
    if last_index >= query.index as usize {
        Ok(())
    } else {
        Err(Error::NoSuchTxInBlock(query.index as usize, last_index))
    }
}

/// Checks that query requests proving for at least one byte of data. Also checks
/// that the queried tx actually contains data at all the indices requested by
/// layout segments.
fn check_queried_bytes_against_txrx(query: &Query, block: &OrderedBlock) -> Result<(), Error> {
    // Get the full length of tx data in bytes
    let payload_len = block
        .items()
        .get(query.index as usize)
        .ok_or(Error::MissingTx)?
        .payload_bytes()
        .len();

    // Should never happen
    if payload_len == 0 {
        return Err(Error::EmptyTxRx);
    }

    // Must have at least one non-empty segment
    if !query.layout_segments.iter().any(|s| s.size > 0) {
        return Err(Error::EmptyQuery);
    }

    // Validate each segment is within bounds: [offset, offset + size) ⊆ [0, payload_len)
    query
        .layout_segments
        .iter()
        .enumerate()
        .try_for_each(|(i, s)| {
            let end = (s.offset as usize)
                .checked_add(s.size as usize)
                .ok_or(Error::OffsetOverflow { segment: i })?;

            if end > payload_len {
                // end is exclusive; comparing to total length avoids off-by-one
                return Err(Error::NoSuchDataInTxRx {
                    segment: i,
                    end,
                    payload_len,
                });
            }
            Ok(())
        })
}
