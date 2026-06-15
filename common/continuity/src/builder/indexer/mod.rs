//! Indexer-based continuity proof building
//!
//! This module contains functions for building continuity proofs using
//! pre-computed data from the GraphQL indexer. These are faster than
//! building from source chain data.

mod chaining;
mod processing;
