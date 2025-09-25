#[cfg(feature = "std")]
pub mod verifier;

#[cfg(feature = "std")]
mod result_segments;

#[cfg(feature = "std")]
mod error;

#[cfg(feature = "std")]
pub use error::VerifierError;

#[cfg(feature = "std")]
pub use verifier::{run_verifier, validate_query_against_proof, NULL_ABI};
