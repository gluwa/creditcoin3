//! GraphQL client for querying the CC3 attestations indexer.
//!
//! This crate provides a client for fetching attestation continuity proofs
//! and related data from the CC3 indexer GraphQL API.

mod client;
mod error;
mod queries;
pub mod types;
mod utils;

#[cfg(test)]
mod tests;

pub use client::IndexerClient;
pub use error::IndexerError;
pub use types::AttestationWithProof;
