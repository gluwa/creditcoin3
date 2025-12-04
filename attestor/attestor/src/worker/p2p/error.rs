use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    PublishError(
        common::types::Height,
        attestor_primitives::Digest,
        libp2p::gossipsub::PublishError,
    ),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::PublishError(height, digest, err) => write!(
                f,
                "Failed to publish local attestation \
                and height {height} \
                with digest {digest}: \
                {err}"
            ),
        }
    }
}

impl std::error::Error for Error {}
