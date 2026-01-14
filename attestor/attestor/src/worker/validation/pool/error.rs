use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    Equivocation(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
    ),
    NoSpaceLeft(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
    ),
    Unauthorized(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
    ),
    InvalidHeight(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
        common::types::Height,
    ),
    InvalidDigest(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
        attestor_primitives::Digest,
    ),
}

impl Error {
    pub fn log_error(self, digest: attestor_primitives::Digest) {
        match self {
            Self::InvalidHeight(_attestor_id, _epoch, height, expected) => {
                tracing::debug!(
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
            Error::Equivocation(address, epoch, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    has already submitted a different vote \
                    at epoch {epoch} \
                    for source chain height {height}"
                )
            }
            Error::NoSpaceLeft(address, epoch, height) => {
                write!(
                    f,
                    "Failed to make more space for vote by attestor {address} \
                    at epoch {epoch} \
                    for source chain height {height}"
                )
            }
            Error::Unauthorized(address, epoch, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    is not part of the validator set \
                    at epoch {epoch} \
                    for source chain height {height}"
                )
            }
            Error::InvalidHeight(address, epoch, height, expected) => {
                write!(
                    f,
                    "Attestor {address} \
                    submitted attestation at epoch {epoch} \
                    for source chain height {height} \
                    but expected height of at least {expected}"
                )
            }
            Error::InvalidDigest(address, epoch, height, digest) => {
                write!(
                    f,
                    "Attestor {address} \
                    submitted attestation at epoch {epoch} \
                    for source chain height {height} \
                    with known invalid digest {digest}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}
