use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    Eth(eth::Error),
    FetchBlock(common::types::Height),
    FetchBlockReceipts(common::types::Height),
    FetchBlockReceiptsMismatch(common::types::Height),
    OrderedBlockConversion(alloy::rpc::types::ConversionError),
    UrlExtractionFailed,
    ReInitError(String),
    StreamError(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Eth(err) => write!(f, "{err}"),
            Error::FetchBlock(height) => write!(
                f,
                "Failed to retreive source chain block at height {height}"
            ),
            Error::FetchBlockReceipts(height) => write!(
                f,
                "Failed to retreive source chain block receipts at height {height}"
            ),
            Error::FetchBlockReceiptsMismatch(height) => write!(
                f,
                "Number of fetched transactions doesn't match number of fetched receipts at height {height}"
            ),
            Error::OrderedBlockConversion(err) => write!(
                f,
                "Failed to convert transaction: {err}"
            ),
            Error::UrlExtractionFailed => write!(
                f,
                "Expected an http url but found a ws. This can't be used to reconstruct an eth client like we need."
            ),
            Error::ReInitError(err) => write!(
                f,
                "Reinitializing the eth client upon chain reversion failed: {err}"
            ),
            Error::StreamError(err) => write!(
                f,
                "Error re-creating stream on reversion or calling next to get head of stream: {err}"
            )
        }
    }
}

impl std::error::Error for Error {}
