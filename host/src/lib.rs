#![cfg_attr(not(feature = "std"), no_std)]

use pallet_prover_primitives::Query;
use sp_core::H256;
use sp_runtime_interface::runtime_interface;
use sp_std::vec::Vec;

#[cfg(feature = "std")]
pub mod command;

#[runtime_interface]
pub trait HostApi {
    fn verify_proof(
        proof: Vec<u8>,
        #[allow(unused)] query: Query,
        #[allow(unused)] metadata: Vec<(u8, H256)>,
    ) -> u8 {
        #[cfg(target_arch = "x86_64")]
        {
            match command::run_verifier(proof, query, metadata) {
                Ok(r) => {
                    log::debug!("result of verifying proof: {:?}", r);
                    0
                }
                Err(e) => command::VerifierError::status_code(&e),
            }
        }

        #[cfg(not(target_arch = "x86_64"))]
        {
            log::debug!("proof len: {}", proof.len());
            log::warn!("run_verifier is not supported on this architecture.");
            0
        }
    }
}

#[runtime_interface]
pub trait HostBenchmarkApi {
    fn verify_proof(_proof: Vec<u8>, query: Query, metadata: Vec<(u8, H256)>) -> bool {
        //benchmark tests are not able to read from file, so we need to substitute the file reading with a hardcoded proof
        let current_path_pwd = std::env::current_exe()
            .expect("should get current path")
            .to_str()
            .expect("should convert to str")
            .to_string();

        let proof_example = current_path_pwd.replace(
            "target/release/creditcoin3-node",
            "cairo/stone-verifier/proof_example.json",
        );

        let proof = std::fs::read(proof_example.clone())
            .unwrap_or_else(|_| panic!("should read file from {}", proof_example));

        match command::run_verifier(proof, query, metadata) {
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
