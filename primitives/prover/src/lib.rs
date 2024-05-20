#![cfg_attr(not(feature = "std"), no_std)]
pub mod claim;

use sp_runtime_interface::runtime_interface;

#[runtime_interface]
pub trait HostApi {
    fn verify_proof(proof: Vec<u8>) -> bool {
        let s = String::from_utf8(proof.clone()).unwrap();
        println!("proof: {:?}", s);

        true
    }
}
