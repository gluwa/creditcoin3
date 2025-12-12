use crate::prelude::*;

#[derive(Debug)]
pub enum Error {
    DoubleVote(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
    ),
    Equivocation(
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
    MissingHeight(
        attestor_primitives::AttestorId,
        common::types::Epoch,
        common::types::Height,
    ),
    MaxBatchSize(
        attestor_primitives::Digest,
        common::types::Epoch,
        common::types::Height,
        u32,
    ),
}

impl Error {
    pub fn log_error(self, digest: attestor_primitives::Digest) {
        match self {
            Self::DoubleVote(_attestor_id, _epoch, height) => {
                tracing::debug!(
                    %digest,
                    height,
                    "Ignoring attestation because it is already stored locally"
                );
            }
            Self::InvalidHeight(_attestor_id, _epoch, height, expected) => {
                tracing::debug!(
                    %digest,
                    height,
                    expected,
                    "Ignoring attestation because it attests to a previous height"
                );
            }
            err => {
                tracing::error!(%err, "⛔ Failed to send remote attestation over for validation");
            }
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::DoubleVote(address, epoch, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    has already voted at epoch {epoch} \
                    for source chain height {height}"
                )
            }
            Error::Equivocation(address, epoch, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    has already submitted a different vote \
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
                    submitted attestation for invalid epoch {epoch} \
                    for source chain height {height}, \
                    expected height of at least {expected}"
                )
            }
            Error::MissingHeight(address, epoch, height) => {
                write!(
                    f,
                    "Failed to remove attestation at epoch {epoch} \
                    for source chain height {height}, \
                    by attestor {address}, \
                    permit points to an empty height"
                )
            }
            Error::MaxBatchSize(digest, epoch, height, max_size) => {
                write!(
                    f,
                    "Attestation batch is full, \
                    failed to append attestation at epoch {epoch} \
                    for source chain height {height} \
                    with digest {digest} ,\
                    max size is {max_size}"
                )
            }
        }
    }
}

impl std::error::Error for Error {}
