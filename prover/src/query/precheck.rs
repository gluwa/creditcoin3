use eth::Client as EthClient;
use thiserror::Error;
use pallet_prover_primitives::Query;
// Steps
// Get source chain block

// Query defects
// - Exceeds max eth tx size -> 128 kb after sanitizing layout segments
// - Query with 0 layout segments
// - Block doesn’t exist yet
// - Tx number not contained in block
// - Selected data out of range contained within Selected tx

#[derive(Debug, Error)]
pub enum Error {
    #[error("Query exceeds maximum size. Byte count: {0}")]
    QuerySizeExceedsMaximum(u64),
    #[error("A query must have at least one byte of data to prove.")]
    EmptyQuery,
    #[error("Query corresponds to non-existant block. Query height: {0}, Highest source block: {0}")]
    Query(u64, u64),
    #[error("No such tx in block. Query tx index: {0}, Max index in block: {0}")]
    NoSuchTxInBlock(u64, u64),
    #[error("No such data in tx. Max data byte index: {0}, Max index contained in tx: {0}")]
    NoSuchDataInTx(u64, u64),
    #[error(transparent)]
    EthError(#[from] eth::Error),
}

pub struct Manager<'a> {
    start_block: u64,
    end_block: u64,
    eth_client: &'a EthClient,
}

impl<'a> Manager<'a> {
    pub fn new(start_block: u64, end_block: u64, eth_client: &'a EthClient) -> Self {
        Self {
            start_block,
            end_block,
            eth_client,
        }
    }
}

/// Checks to make sure that a query isn't obviously invalid before processing it.
/// - `query`: A query awaiting processing
/// - `eth_client`: Ethereum client for source chain
pub(crate) async fn pre_check_query(query: Query, eth_client: &EthClient) -> Result<(), Error> {
    let query_block = eth_client.get_block(query.height).await?;
    Ok(())
}
