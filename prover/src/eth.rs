// use alloy::providers::{Provider, ProviderBuilder};
use thiserror::Error;
use tracing::error;

// use crate::{
// cc3::{self},
// transaction,
// };

#[derive(Debug, Error)]
pub enum Error {
    #[error("Failed to get block {0}")]
    FailedToGetBlock(u64),
}
