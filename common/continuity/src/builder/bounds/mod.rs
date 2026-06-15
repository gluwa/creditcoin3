//! Attestation and checkpoint bounds finding logic
//!
//! This module provides abstractions and implementations for finding optimal
//! attestation/checkpoint bounds when building continuity proofs.
//!
//! Two implementations are provided:
//! - `cc3`: Direct CC3 chain queries (slower but works without indexer)
//! - `indexer`: GraphQL indexer queries (faster, requires indexer)

mod cc3;
mod indexer;

pub use cc3::Cc3BoundsFinder;
pub use indexer::IndexerBoundsFinder;

use crate::errors::ContinuityError;
use async_trait::async_trait;
use indexer_client::AttestationWithProof;

/// Trait for finding attestation/checkpoint bounds for continuity proof generation.
///
/// Different implementations can use different data sources (CC3 chain, indexer, etc.)
/// but all follow the same logic: checkpoints first (permanent), then attestations.
#[async_trait]
pub trait BoundsFinder {
    /// Find the lower and upper bounds for a query range.
    ///
    /// Returns:
    /// - Lower bound: Attestation/checkpoint at or before `min_query - 1`
    /// - Upper bound: Attestation/checkpoint after `max_query`
    async fn find_bounds(
        &self,
        min_query: u64,
        max_query: u64,
        current_block: Option<u64>,
    ) -> Result<(AttestationWithProof, AttestationWithProof), ContinuityError>;
}
