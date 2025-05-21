use sp_core::H256;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

use crate::continuity::Error as ContinuityError;
use attestor_primitives::Attestation;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Failed to submit attestation")]
    FailedToSubmit,
    #[error("Double vote")]
    DoubleVote,
    #[error("Engine is not running")]
    NotRunning,
    #[error("cclient error: {0}")]
    Cclient(#[from] cc_client::Error),
    #[error("Attestor not selected for header {0}")]
    NotSelected(u64),
    #[error("Failed to get chain key")]
    FailedToGetChainKey,
    #[error("Failed to get chain name")]
    FailedToGetChainName,
    #[error("Failed to get attestation interval")]
    FailedToGetAttestationInterval,
    #[error("Invalid BLS key")]
    InvalidBlsKey,
    #[error("Invalid proof of possession")]
    InvalidProofOfPossession,
    #[error("Failed to subscribe {0}")]
    FailedToSubscribe(String),
    #[error("Eth client error {0}")]
    EthClient(#[from] eth::Error),
    #[error("Send error {0}")]
    Send(#[from] SendError<Attestation<H256>>),
    #[error("Continuity Error {0}")]
    Continuity(#[from] ContinuityError),
    #[error("Other error: {0}")]
    Other(#[from] anyhow::Error),
}

impl Error {
    #[must_use]
    pub fn is_not_selected_error(&self) -> bool {
        matches!(
            self,
            Error::Cclient(cc_client::Error::FailedToCreateProofOfInclusion(_))
        ) || matches!(self, Error::NotSelected(_))
    }

    #[must_use]
    pub fn is_fragment_error(&self) -> bool {
        matches!(self, Error::Continuity(_))
    }

    #[must_use]
    pub fn is_not_running_error(&self) -> bool {
        matches!(self, Error::NotRunning)
    }

    #[must_use]
    pub fn is_double_vote_error(&self) -> bool {
        matches!(self, Error::DoubleVote)
    }

    #[must_use]
    pub fn is_client_error(&self) -> bool {
        matches!(self, Error::Cclient(cc_client::Error::SubxtError(_)))
    }
}
