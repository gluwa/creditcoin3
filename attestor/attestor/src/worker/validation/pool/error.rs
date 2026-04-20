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
            Self::InvalidHeight(attestor_id, height, last_finalized) => {
                tracing::debug!(
                    %attestor_id,
                    ?digest,
                    height,
                    last_finalized,
                    "Ignoring attestation with inadmissible height \
                    (at or below finalized, or beyond catch-up window)"
                );
            }
            Self::InvalidDigest(attestor_id, height, known_invalid_digest) => {
                tracing::debug!(
                    %attestor_id,
                    ?digest,
                    height,
                    ?known_invalid_digest,
                    "Ignoring attestation with a known invalid digest"
                );
            }
            Self::NoSpaceLeft(attestor_id, height) => {
                tracing::error!(
                    %attestor_id,
                    ?digest,
                    height,
                    "⛔ Pool is full, vote dropped"
                );
            }
            Self::Equivocation(attestor_id, height) => {
                tracing::error!(
                    %attestor_id,
                    ?digest,
                    height,
                    "⛔ Equivocation detected: attestor already voted at this height with a different digest"
                );
            }
            Self::Unauthorized(attestor_id, height) => {
                tracing::error!(
                    %attestor_id,
                    ?digest,
                    height,
                    "⛔ Unauthorized attestor vote rejected"
                );
            }
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Equivocation(address, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    has already submitted a different vote \
                    for source chain height {height}"
                )
            }
            Self::NoSpaceLeft(address, height) => {
                write!(
                    f,
                    "Failed to make more space for vote by attestor {address} \
                    for source chain height {height}"
                )
            }
            Self::Unauthorized(address, height) => {
                write!(
                    f,
                    "Attestor {address} \
                    is not part of the validator set \
                    for source chain height {height}"
                )
            }
            Self::InvalidHeight(address, height, last_finalized) => {
                write!(
                    f,
                    "Attestor {address} \
                    submitted vote at inadmissible height {height} \
                    (last finalized: {last_finalized})"
                )
            }
            Self::InvalidDigest(address, height, digest) => {
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
