use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    SubxtError(subxt::Error),
    Cc3Client(cc_client::Error),
    EthClient(eth::Error),
    AttestationAlreadyAttestedTo(common::types::Height),
    AttestationContinuityError(eth::continuity::Error),
    AttestationInvalidRange(common::types::Height, common::types::Height),
    SubscriptionEnd,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::SubxtError(err) => write!(f, "{err}"),
            Error::Cc3Client(err) => write!(f, "{err}"),
            Error::EthClient(err) => write!(f, "{err}"),
            Error::AttestationAlreadyAttestedTo(height) => write!(
                f,
                "Source chain block at height {height} has already been attested to"
            ),
            Error::AttestationContinuityError(err) => write!(f, "{err}"),
            Error::AttestationInvalidRange(start, end) => write!(
                f,
                "Failed to create attestation fragment on invalid range [{start}, {end}]"
            ),
            Error::SubscriptionEnd => write!(f, "Unexpected end of stream"),
        }
    }
}

impl std::error::Error for Error {}
