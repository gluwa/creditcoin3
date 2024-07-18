#![cfg_attr(not(feature = "std"), no_std)]

use sp_runtime_interface::runtime_interface;
use sp_std::vec::Vec;

#[cfg(feature = "std")]
pub mod command;

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

#[runtime_interface]
pub trait HostBenchmarkApi {
    fn verify_proof(_proof: Vec<u8>) -> bool {
        //benchmark tests are not able to read from file, so we need to substitute the file reading with a hardcoded proof
        let current_path_pwd = std::env::current_exe()
            .expect("should get current path")
            .to_str()
            .expect("should convert to str")
            .to_string();

        let proof_example = current_path_pwd.replace(
            "target/release/creditcoin3-node",
            "host/stone-verifier/proof_example.json",
        );

        let proof = std::fs::read(proof_example.clone())
            .unwrap_or_else(|_| panic!("should read file from {}", proof_example));

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
