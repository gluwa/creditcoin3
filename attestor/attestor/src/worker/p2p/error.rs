use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    Client(cc_client::Error),
    Subxt(subxt::Error),
    InvalidAttestation(InvalidCause),
    PublishError(
        common::types::Height,
        attestor_primitives::Digest,
        libp2p::gossipsub::PublishError,
    ),
    Transport(libp2p::TransportError<std::io::Error>),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Client(err) => write!(f, "{err}"),
            Self::Subxt(err) => write!(f, "{err}"),
            Self::InvalidAttestation(cause) => write!(f, "Invalid attestation: {cause}"),
            Self::PublishError(height, digest, err) => write!(
                f,
                "Failed to publish local attestation \
                and height {height} \
                with digest {digest}: \
                {err}"
            ),
            Self::Transport(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
pub enum InvalidCause {
    InvalidBls,
    Unregistered,
}

impl std::fmt::Display for InvalidCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidBls => write!(f, "Invalid attestation BLS"),
            Self::Unregistered => write!(f, "Attestor is not registered on-chain"),
        }
    }
}
