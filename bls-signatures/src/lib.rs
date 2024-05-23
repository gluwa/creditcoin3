#![no_std]

// #[cfg(all(feature = "pairing", feature = "blst"))]
// compile_error!("only pairing or blst can be enabled");

mod error;
pub mod key;
mod signature;

pub use self::error::Error;
pub use self::key::{PrivateKey, PublicKey, Serialize};
pub use self::signature::{
    aggregate, hash, verify, verify_aggregated_signatures_on_same_message, verify_messages,
    Signature,
};

#[cfg(test)]
extern crate base64_serde;
