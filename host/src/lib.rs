#![cfg_attr(not(feature = "std"), no_std)]

use sp_runtime_interface::runtime_interface;
use sp_std::vec::Vec;

#[cfg(feature = "std")]
pub mod nix;

#[runtime_interface]
pub trait HostApi {
    fn verify_proof(proof: Vec<u8>) -> bool {
        let result = nix::call_verifier(proof);
        println!("result of verifying proof: {:?}", result);

        true
    }
}

