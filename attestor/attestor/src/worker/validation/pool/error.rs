use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    Equivocation(attestor_primitives::AttestorId, common::types::Height),
    NoSpaceLeft(attestor_primitives::AttestorId, common::types::Height),
    Unauthorized(attestor_primitives::AttestorId, common::types::Height),
    InvalidHeight(
        attestor_primitives::AttestorId,
        common::types::Height,
        common::types::Height,
    ),
    InvalidDigest(
        attestor_primitives::AttestorId,
        common::types::Height,
        attestor_primitives::Digest,
    ),
}

impl Error {
    pub fn log_error(self, digest: attestor_primitives::Digest) {
        match self {
            Self::InvalidHeight(attestor_id, height, expected) => {
                tracing::debug!(
                    %attestor_id,
                    %digest,
                    height,
                    expected,
                    "Ignoring attestation because it attests to a previous height"
                );
            }
            err => {
                tracing::error!(%err, "⛔ Failed to insert vote into the attestation pool");
            }
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Equivocation(address, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    has already submitted a different vote \
                    for source chain height {height}"
                )
            }
            Error::NoSpaceLeft(address, height) => {
                write!(
                    f,
                    "Failed to make more space for vote by attestor {address} \
                    for source chain height {height}"
                )
            }
            Error::Unauthorized(address, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    is not part of the validator set \
                    for source chain height {height}"
                )
            }
            Error::InvalidHeight(address, height, expected) => {
                write!(
                    f,
                    "Attestor {address} \
                    for source chain height {height} \
                    but expected height of at least {expected}"
                )
            }
            Error::InvalidDigest(address, height, digest) => {
                write!(
                    f,
                    "Attestor {address} \
                    for source chain height {height} \
                    with known invalid digest {digest}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}
