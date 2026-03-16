//! # Continuity Proof Generation Library
//!
//! This crate provides the core logic for building continuity proofs that cryptographically
//! link source chain blocks to Creditcoin3 attestations.
//!
//! ## Overview
//!
//! Continuity proofs allow on-chain verification that a specific block on a source chain
//! (e.g., Ethereum) was correctly attested to by the Creditcoin3 network. The proof consists
//! of a chain of blocks that links the query block to attestation consensus points.
//!
//! ## Quick Start
//!
//! See [`ContinuityBuilder`] for usage examples.
//!
//! ## Data Sources
//!
//! The builder supports two data sources with automatic fallback:
//!
//! 1. **Indexer (Fast)** - Fetches pre-computed proofs from the indexer GraphQL API
//! 2. **CC3 Chain (Fallback)** - Builds proofs from CC3 chain queries and source chain data
//!
//! ## Features
//!
//! - `block_cache` - Enable Redis-based block caching for improved performance
//!
//! ## Main Types
//!
//! - [`ContinuityBuilder`] - Main entry point for building proofs
//! - [`ContinuityConfig`] - Configuration for the builder
//! - [`BuiltContinuityProof`] - The resulting proof structure (precomputed from indexer data)
//! - [`AttestationWithProof`] - Attestation boundary information with optional proof data
//! - [`ContinuityError`] - Error types with detailed context

#[cfg(feature = "archiver")]
pub mod archiver;
pub mod builder;
pub mod config;
pub mod errors;
pub mod mocks;
pub mod proof;
pub mod rpc;

pub use builder::{ContinuityBuilder, ContinuityResult};
pub use config::{ConfigBuilder, ContinuityConfig};
pub use errors::ContinuityError;
pub use indexer_client::AttestationWithProof;
pub use proof::BuiltContinuityProof;
pub use rpc::{CcRpcProvider, EthRpcProvider, SharedCcProvider, SharedEthProvider};
