#![no_std]

mod error;
pub mod key;
mod signature;

pub use self::error::Error;
pub use self::key::{PrivateKey, PublicKey, Serialize};
pub use self::signature::{
    aggregate, hash, verify, verify_agg_message, verify_messages, Signature,
};

#[cfg(test)]
extern crate base64_serde;
