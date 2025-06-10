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

/// Solidity selector of the ProofSubmitted log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_PROOF_SUBMITTED: [u8; 32] = keccak256!("ProofSubmitted(address, bytes32)");

/// Precompile exposing a pallet_balance as an ERC20.
/// The precompile uses an additional storage to store approvals.
pub struct ProofVerifierPrecompile<Runtime>(PhantomData<Runtime>);

type ConstU50MB = sp_core::ConstU32<52428800>;

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

        let proof_bytes: Vec<u8> = proof.into();

        if proof_bytes.is_empty() {
            error!("Empty proof submitted");
            let encoded_revert = encode_revert_message("Invalid proof submitted");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        let metadata: Vec<(u8, H256)> = StarkProgramMetadata::<Runtime>::iter().collect();
        if metadata.is_empty() {
            error!("Verification failed: Stark program metadata not set");
            let encoded_revert = encode_revert_message("Stark program metadata not set");
            return Err(PrecompileFailure::Revert {
                output: encoded_revert,
                exit_status: ExitRevert::Reverted,
            });
        }

        #[cfg(not(feature = "runtime-benchmarks"))]
        {
            let (status, result_segments, continuity_proof_len, continuity_checkpoint_digest) =
                proof_verifier::host_api::verify_proof(proof_bytes, query.clone(), metadata);

            let status = Self::handle_error_status(
                status,
                continuity_proof_len,
                continuity_checkpoint_digest,
                &query,
            )?;

            info!("Proof verification completed for query: {:?}", query.id());
            info!("Proof verification status: {}", status);
            info!("Result segments: {:?}", result_segments);

            log3(
                handle.context().address,
                SELECTOR_LOG_PROOF_SUBMITTED,
                handle.context().caller,
                query.id(),
                solidity::encode_event_data(result_segments.clone()),
            )
            .record(handle)?;

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
    ) -> Result<(u64, H256), PrecompileFailure> {
        if continuity_proof_len.is_none() || continuity_checkpoint_digest.is_none() {
            error!("Missing continuity proof or checkpoint digest");
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
        continuity_proof_len: u64,
        continuity_checkpoint_digest: H256,
        query: &Query,
    ) -> Result<(), PrecompileFailure> {
        let checkpoint_block_number = query.height - 1 + continuity_proof_len - 1;

        let expected_block_number = if let Some(last_checkpoint_number) =
            <Runtime as pallet_prover::Config>::Checkpoints::get_last_checkpoint_number(
                query.chain_id,
            ) {
            // Use a checkpoint if one is available
            if last_checkpoint_number >= query.height {
                // Fetch checkpoint block number
                Self::get_block_number_or_revert(
                    query.chain_id,
                    continuity_checkpoint_digest,
                    true,
                )?
            } else {
                // Fetch attestation if last checkpoint is before the query height
                Self::get_block_number_or_revert(
                    query.chain_id,
                    continuity_checkpoint_digest,
                    false,
                )?
            }
        } else {
            // Fetch attestation if no checkpoints are available
            Self::get_block_number_or_revert(query.chain_id, continuity_checkpoint_digest, false)?
        };

        if checkpoint_block_number != expected_block_number {
            error!(
                "Continuity proof block number mismatch: expected {}, got {}",
                expected_block_number, checkpoint_block_number
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
    fn get_block_number_or_revert(
        chain_id: u64,
        continuity_checkpoint_digest: H256,
        fetch_checkpoint: bool,
    ) -> Result<u64, PrecompileFailure> {
        let result = if fetch_checkpoint {
            <Runtime as pallet_prover::Config>::Checkpoints::get_checkpoint(
                chain_id,
                continuity_checkpoint_digest,
            )
            .map(Ok)
            .unwrap_or_else(|| Err("Continuity Checkpoint digest not found"))
        } else {
            <Runtime as pallet_prover::Config>::Attestations::get_attestation(
                chain_id,
                continuity_checkpoint_digest,
            )
            .map(|att| Ok(att.header_number()))
            .unwrap_or_else(|| Err("Continuity Attestation digest not found"))
        };

        match result {
            Ok(val) => Ok(val),
            Err(msg) => {
                error!("{}", msg);
                let encoded_revert = encode_revert_message(msg);
                Err(PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                })
            }
        }
    }

    #[cfg(not(feature = "runtime-benchmarks"))]
    fn handle_error_status(
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
                    )?;

                Self::check_continuity_block_number(
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

                error!("{}", error_msg);
                let encoded_revert = encode_revert_message(error_msg);

                Err(PrecompileFailure::Revert {
                    output: encoded_revert,
                    exit_status: ExitRevert::Reverted,
                })
            }
        }
    }
}
