use thiserror::Error;

/// Domain-level continuity errors.
///
/// These are intentionally kept inside the continuity crate and NOT directly surfaced
/// by the HTTP layer. The API server maps internal failures to `ServiceError` variants
/// at the boundary. If/when builder & RPC logic starts returning these explicitly,
/// the mapping can be (re)introduced in the server crate. For now, avoiding an unused
/// cross-crate conversion keeps the error surface minimal.

#[derive(Debug, Error)]
pub enum ContinuityError {
    #[error("No attestations available for chain_key {0}")]
    NoAttestations(u64),

    #[error("Internal continuity error: {0}")]
    Internal(String),

    #[error("RPC error: {0}")]
    Rpc(String),

    #[error("Invalid attestation bounds: {0}")]
    InvalidBounds(String),

    #[error("Block not found in continuity chain")]
    MissingBlock,
}
