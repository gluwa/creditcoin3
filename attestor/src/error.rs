use eth::OrderedBlock;
use thiserror::Error;
use tokio::sync::mpsc::error::SendError;

use crate::continuity::Error as ContinuityError;
use cc_client::attestation::Error as AttestationError;

#[derive(Error, Debug)]
pub enum Error {
    #[error("cclient error: {0}")]
    Cclient(#[from] cc_client::Error),
    #[error("Attestor not selected for header {0}")]
    NotSelected(u64),
    #[error("Failed to get chain key")]
    FailedToGetChainKey,
    #[error("Wrong chain: expected {0}, got {1}")]
    WrongChain(u64, u64),
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
    Send(#[from] SendError<OrderedBlock>),
    #[error("Continuity Error {0}")]
    Continuity(#[from] ContinuityError),
    #[error("Block already attested to: {0}")]
    AlreadyAttestedTo(u64),
    #[error("CcClient error: {0}")]
    AttestationClient(#[from] AttestationError),
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
    pub fn is_attested_to_error(&self) -> bool {
        matches!(self, Error::AlreadyAttestedTo(_))
    }
}
