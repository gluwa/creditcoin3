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
//! ```rust,no_run
//! use continuity::{ContinuityBuilder, ContinuityConfig};
//! # async fn example() -> anyhow::Result<()> {
//!
//! // Create configuration (automatically fetches checkpoint_interval from CC3 chain)
//! let config = ContinuityConfig::builder()
//!     .cc3_rpc_url("wss://rpc.creditcoin.network")
//!     .eth_rpc_url("https://eth-rpc.example.com")
//!     .chain_key(1)
//!     .fetch_checkpoint_interval()
//!     .await?;
//!
//! // Build the continuity builder
//! let builder = ContinuityBuilder::new(config).await?;
//!
//! // Get attestation endpoints and build proof
//! let query_height = 100;
//! let (lower, upper, _) = builder.get_endpoints(&[query_height], None).await?;
//! let proof = builder.build_for_single_query(query_height, lower, upper).await?;
//!
//! println!("Generated proof with {} blocks", proof.blocks.len());
//! # Ok(())
//! # }
//! ```
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

pub mod builder;
pub mod config;
pub mod errors;
pub mod mocks;
pub mod proof;
pub mod rpc;

pub use builder::{ContinuityBuilder, ContinuityResult, EndsInAttestation};
pub use config::{ConfigBuilder, ContinuityConfig};
pub use errors::ContinuityError;
pub use indexer_client::AttestationWithProof;
pub use proof::BuiltContinuityProof;
pub use rpc::{CcRpcProvider, EthRpcProvider, SharedCcProvider, SharedEthProvider};
