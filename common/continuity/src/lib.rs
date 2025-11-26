//! Continuity Proof - Shared Logic Crate
//!
//! This crate provides common types and behavior for building continuity proofs.
//! It is shared between:
//! - query-cli
//! - proof-gen-api-server
//!
//! Continuity proof generation – shared library used by query-cli & API server.

pub mod attestation;
pub mod config;
pub mod errors;
pub mod proof;

pub mod builder;
pub mod mock_providers;
pub mod rpc;

pub use attestation::AttestationInfo;
pub use builder::ContinuityBuilder;
pub use config::ContinuityConfig;
pub use mock_providers::make_mock_providers;
pub use proof::ContinuityProof;
pub use rpc::{CcRpcProvider, EthRpcProvider};
