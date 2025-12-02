use thiserror::Error;

#[derive(Debug, Error, Clone)]
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

    #[error("The continuity proof cannot be created because block {block_number} is not attested to yet")]
    BlockNotReady {
        block_number: u64,
        current_block: u64,
    },
}
