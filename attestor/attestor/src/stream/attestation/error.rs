use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    Interrupt,
    Eth(eth::Error),
    FetchBlock(common::types::Height),
    FetchBlockReceipts(common::types::Height),
    FetchBlockReceiptsMismatch(common::types::Height),
    OrderedBlockConversion(alloy::rpc::types::ConversionError),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Interrupt => todo!(),
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
        }
    }
}

impl std::error::Error for Error {}
