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

    #[error("The continuity proof cannot be created because block {block_number} is not attested to yet. Last attested block: {last_attested_block}")]
    BlockNotReady {
        block_number: u64,
        last_attested_block: u64,
    },

    #[error("Block {requested_block} is before attestation genesis block {genesis_block}. Cannot generate proofs for blocks before the attestation system was initialized.")]
    BlockBeforeGenesis {
        requested_block: u64,
        genesis_block: u64,
    },

    #[error("No consensus point (attestation or checkpoint) found before block {block_number}. Cannot build continuity proof.")]
    NoConsensusPointBefore { block_number: u64 },

    #[error("Attestation interval not configured for chain_key {chain_key}. Cannot predict upper bound.")]
    AttestationIntervalNotConfigured { chain_key: u64 },

    #[error("Empty query: no block heights provided")]
    EmptyQuery,
}
