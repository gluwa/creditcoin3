#![cfg_attr(not(feature = "std"), no_std)]

use sp_runtime_interface::runtime_interface;
use sp_std::vec::Vec;

#[cfg(feature = "std")]
pub mod command;

#[cfg(verify_proof)]
#[runtime_interface]
pub trait HostApi {
    fn verify_proof(proof: Vec<u8>) -> bool {
        match command::run_verifier(proof) {
            Ok(r) => {
                log::debug!("result of verifying proof: {:?}", r);
                true
            }
            Err(e) => {
                log::error!("error verifying proof: {:?}", e);
                false
            }
        }
    }
}

#[cfg(not(verify_proof))]
#[runtime_interface]
pub trait HostApi {
    fn verify_proof(_proof: Vec<u8>) -> bool {
        true
    }
}
