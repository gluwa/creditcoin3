#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "runtime-benchmarks"))]
use attestor_primitives::provider::{AttestationProvider, CheckpointProvider};
use core::marker::PhantomData;
use ethabi::{encode, Token};
use fp_evm::{ExitRevert, PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::{error, info};
use pallet_evm::AddressMapping;
use pallet_prover::StarkProgramMetadata;
use pallet_prover_primitives::{Query, ResultSegment};
use precompile_utils::{prelude::*, solidity::Codec};
use sp_core::H256;
use sp_std::vec::Vec;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Precompile exposing a pallet_balance as an ERC20.
/// The precompile uses an additional storage to store approvals.
pub struct ProofVerifierPrecompile<Runtime>(PhantomData<Runtime>);

type ConstU50MB = sp_core::ConstU32<52428800>;

const GAS_BASE_VERIFY: u64 = 50_000; // Base overhead for entering the precompile
const GAS_PER_PROOF_BYTE: u64 = 10; // Per byte Gas for the proof
const STARK_META_VALUE_ENCODED_LEN: usize = 32; // H256 via SCALE => 32 bytes
const WEIGHT_STARK_VERIFY: u64 = 5_000_000; // STARK verification heavy cost (fixed for now)
const GAS_STORAGE_LOOKUP: u64 = 5_000; // Each storage read

fn encode_revert_message(message: &str) -> Vec<u8> {
    // function selector for Error(string)
    let mut revert_with_selector = [0x08, 0xc3, 0x79, 0xa0].to_vec();

    let encoded_revert = encode(&[Token::String(message.into())]);
    revert_with_selector.extend(encoded_revert);

    revert_with_selector
}

#[derive(Debug, Clone, PartialEq, Eq, Codec)]
pub struct VerifyResult {
    pub status: u8, // 0: Success, 1: ProofInvalid, 2: LayoutMismatch, 3: OutOfBounds
    pub result_segments: Vec<ResultSegment>,
}

#[precompile_utils::precompile]
impl<Runtime> ProofVerifierPrecompile<Runtime>
where
    Runtime: pallet_prover::Config + pallet_evm::Config,
    Runtime::Hash: Into<H256>,
    H256: Into<Runtime::Hash>,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_prover::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    #[precompile::public("verify(bytes,(uint64,uint64,uint64,(uint64,uint64)[]))")]
    fn verify(
        handle: &mut impl PrecompileHandle,
        proof: BoundedBytes<ConstU50MB>,
        query: Query,
    ) -> EvmResult<VerifyResult> {
        handle.record_log_costs_manual(3, 32)?;
        // Base cost for invoking the precompile
        handle.record_cost(GAS_BASE_VERIFY)?;

        let proof_bytes: Vec<u8> = proof.into();

        handle.record_cost(GAS_PER_PROOF_BYTE.saturating_mul(proof_bytes.len() as u64))?;

        if proof_bytes.is_empty() {
            error!("Empty proof submitted. QueryId: {:?}", query.id());
            let encoded_revert = encode_revert_message("Invalid proof submitted");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        let metadata: Vec<(u8, H256)> = StarkProgramMetadata::<Runtime>::iter().collect();
        for _ in &metadata {
            // charge one DB read
            handle.record_db_read::<Runtime>(STARK_META_VALUE_ENCODED_LEN)?;
        }

        if metadata.is_empty() {
            error!(
                "Verification failed: Stark program metadata not set, QueryId: {:?}",
                query.id()
            );
            let encoded_revert = encode_revert_message("Stark program metadata not set");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        // Charge fixed weight for the STARK verification work (converted to gas using WeightPerGas)
        let w = sp_weights::Weight::from_parts(WEIGHT_STARK_VERIFY, 0);
        RuntimeHelper::<Runtime>::record_external_cost(handle, w, 0)?;

        #[cfg(not(feature = "runtime-benchmarks"))]
        {
            let (status, result_segments, continuity_proof_len, continuity_checkpoint_digest) =
                proof_verifier::host_api::verify_proof(proof_bytes, query.clone(), metadata);

            let status = Self::handle_error_status(
                handle,
                status,
                continuity_proof_len,
                continuity_checkpoint_digest,
                &query,
            )?;

            info!(
                "Proof verification completed for query: {:?}, status: {}",
                query.id(),
                status
            );
            log::debug!("Result segments: {result_segments:?}");

            Ok(VerifyResult {
                status,
                result_segments,
            })
        }

        #[cfg(feature = "runtime-benchmarks")]
        {
            let result = proof_verifier::host_benchmark_api::verify_proof(
                proof_bytes,
                query.clone(),
                metadata,
            );
            handle.record_cost(GAS_STORAGE_LOOKUP)?;
            if !result {
                error!("Proof verification failed: Invalid proof submitted");
                let encoded_revert = crate::encode_revert_message("Invalid proof submitted");
                Err(PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                })
            } else {
                info!("Proof verification completed for query: {:?}", query.id());
                Ok(VerifyResult {
                    status: 0,
                    result_segments: sp_std::vec![],
                })
            }
        }
    }

    #[cfg(not(feature = "runtime-benchmarks"))]
    fn check_continuity_proof(
        continuity_proof_len: Option<u64>,
        continuity_checkpoint_digest: Option<H256>,
        query_id: H256,
    ) -> Result<(u64, H256), PrecompileFailure> {
        if continuity_proof_len.is_none() || continuity_checkpoint_digest.is_none() {
            error!("Missing continuity proof or checkpoint digest. QueryId: {query_id:?}",);
            let encoded_revert =
                encode_revert_message("Missing continuity proof or checkpoint digest");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        Ok((
            continuity_proof_len.unwrap(),
            continuity_checkpoint_digest.unwrap(),
        ))
    }

    #[cfg(not(feature = "runtime-benchmarks"))]
    fn check_continuity_block_number(
        handle: &mut impl PrecompileHandle,
        continuity_proof_len: u64,
        continuity_checkpoint_digest: H256,
        query: &Query,
    ) -> Result<(), PrecompileFailure> {
        let checkpoint_block_number = query.height - 1 + continuity_proof_len - 1;

        // Always try to get a matching attestation first. The proof may have been generated using an
        // attestation, even if there is now a checkpoint at that height.
        // Charge for attestation storage lookup
        handle.record_cost(GAS_STORAGE_LOOKUP)?;
        let expected_block_number = if let Some(matching_attestation) =
            <Runtime as pallet_prover::Config>::Attestations::get_attestation(
                query.chain_id,
                continuity_checkpoint_digest,
            ) {
            matching_attestation.attestation.header_number
        } else {
            // On error, try to get a matching checkpoint instead
            // Charge for checkpoint storage lookup
            handle.record_cost(GAS_STORAGE_LOOKUP)?;
            if let Some(number) = <Runtime as pallet_prover::Config>::Checkpoints::get_checkpoint(
                query.chain_id,
                continuity_checkpoint_digest,
            ) {
                number
            } else {
                let message = "Continuity digest doesn't match any attestation or checkpoint";
                error!("{}, QueryId: {:?}", message, query.id());
                let encoded_revert = encode_revert_message(message);
                return Err(PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                });
            }
        };

        if checkpoint_block_number != expected_block_number {
            error!(
                "Continuity proof block number mismatch: expected {}, got {}. QueryId: {:?}",
                expected_block_number,
                checkpoint_block_number,
                query.id()
            );
            let encoded_revert = encode_revert_message("Continuity proof block number mismatch");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        };

        Ok(())
    }

    #[cfg(not(feature = "runtime-benchmarks"))]
    fn handle_error_status(
        handle: &mut impl PrecompileHandle,
        status: u8,
        continuity_proof_len: Option<u64>,
        continuity_checkpoint_digest: Option<H256>,
        query: &Query,
    ) -> Result<u8, PrecompileFailure> {
        match status {
            0 => {
                let (continuity_proof_len, continuity_checkpoint_digest) =
                    Self::check_continuity_proof(
                        continuity_proof_len,
                        continuity_checkpoint_digest,
                        query.id(),
                    )?;

                Self::check_continuity_block_number(
                    handle,
                    continuity_proof_len,
                    continuity_checkpoint_digest,
                    query,
                )?;

                Ok(0)
            }
            _ => {
                let error_msg = match status {
                    1..=7 => "Proof verification failed: ProcessError",
                    8 => "Proof verification failed: StarkMetadataMismatch",
                    12 => "Proof verification failed: QueryOutOfBounds",
                    13 => "Proof verification failed: QueryOffsetsMismatch",
                    17 => "Proof verification failed: QueryLayoutSegmentsError",
                    18 => "Proof verification failed: QueryTransactionIdMismatch",
                    _ => "Proof verification failed: InvalidProofSubmitted",
                };

                error!(
                    "Error verifying query. Error: {}, QueryId: {:?}",
                    error_msg,
                    query.id()
                );
                let encoded_revert = encode_revert_message(error_msg);

                Err(PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                })
            }
        }
    }
}
