//! Error types for continuity proof generation.
//!
//! This module defines all errors that can occur during proof generation,
//! with detailed context for debugging and user-facing error messages.

use attestor_primitives::ChainKey;
use thiserror::Error;

/// Errors that can occur during continuity proof generation.
///
/// These errors are designed to provide clear, actionable information about
/// what went wrong and whether the operation can be retried.
#[derive(Debug, Error, Clone)]
pub enum ContinuityError {
    /// No attestations exist for the specified chain.
    ///
    /// This typically means the attestation system hasn't started yet for this chain,
    /// or the chain_key is invalid.
    #[error("No attestations available for chain_key {0}")]
    NoAttestations(u64),

    /// An internal error occurred in the continuity building logic.
    ///
    /// This shouldn't happen in normal operation and indicates a bug.
    #[error("Internal continuity error: {0}")]
    Internal(String),

    /// An RPC call to CC3 or the source chain failed.
    ///
    /// This could be due to network issues, RPC endpoint unavailability,
    /// or invalid responses.
    #[error("RPC error: {0}")]
    Rpc(String),

    /// The attestation bounds are invalid for the given query.
    ///
    /// This can happen if the lower bound is after the upper bound,
    /// or if the bounds don't properly contain the query blocks.
    #[error("Invalid attestation bounds: {0}")]
    InvalidBounds(String),

    /// A required block was not found in the continuity chain.
    #[error("Block not found in continuity chain")]
    MissingBlock,

    /// The requested block hasn't been attested yet.
    ///
    /// This is a **retriable** error - the client should wait for the next
    /// attestation and try again.
    #[error("The continuity proof cannot be created because block {block_number} is not attested to yet. Last attested block: {last_attested_block}")]
    BlockNotReady {
        block_number: u64,
        last_attested_block: u64,
    },

    /// The requested block is before the attestation genesis block.
    ///
    /// Proofs cannot be generated for blocks before the attestation system
    /// was initialized on the Creditcoin3 chain.
    #[error("Block {requested_block} is before attestation genesis block {genesis_block}. Cannot generate proofs for blocks before the attestation system was initialized.")]
    BlockBeforeGenesis {
        requested_block: u64,
        genesis_block: u64,
    },

    /// No attestation or checkpoint exists before the requested block.
    ///
    /// A continuity proof requires a consensus point before the query block
    /// to serve as the lower boundary.
    #[error("No consensus point (attestation or checkpoint) found before block {block_number}. Cannot build continuity proof.")]
    NoConsensusPointBefore { block_number: u64 },

    /// The attestation interval is not configured for this chain.
    ///
    /// This prevents the builder from predicting the next attestation block
    /// when building "eager" proofs.
    #[error("Attestation interval not configured for chain_key {chain_key}. Cannot predict upper bound.")]
    AttestationIntervalNotConfigured { chain_key: ChainKey },

    /// No query heights were provided.
    #[error("Empty query: no block heights provided")]
    EmptyQuery,

    /// The predicted upper attestation bound doesn't exist on the source chain yet.
    ///
    /// This is a **retriable** error - the source chain needs more blocks to be mined
    /// before the proof can be generated.
    #[error("Cannot build continuity proof yet: predicted upper attestation bound (block {upper_block}) does not exist on the source chain yet. Current source chain height: {current_block}. Query block {query_block} exists, but the proof requires the next attestation block which hasn't been mined yet.")]
    UpperBoundNotOnSourceChain {
        query_block: u64,
        upper_block: u64,
        current_block: u64,
    },
}

impl ContinuityError {
    /// Check if this error is retriable.
    ///
    /// Returns `true` if the client should wait and retry the operation,
    /// `false` if retrying won't help.
    pub fn is_retriable(&self) -> bool {
        matches!(
            self,
            ContinuityError::BlockNotReady { .. }
                | ContinuityError::UpperBoundNotOnSourceChain { .. }
                | ContinuityError::Rpc(_)
        )
    }

    /// Get a user-friendly error message.
    pub fn user_message(&self) -> String {
        self.to_string()
    }
}
