#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::{PrecompileFailure, PrecompileHandle};
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::error;
use pallet_evm::AddressMapping;
use pallet_prover::ResultSegmentsById;
use pallet_prover_primitives::{Query, ResultSegment};
use precompile_utils::prelude::*;
use sp_core::H256;
use sp_runtime::{format, DispatchError};
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
    ) -> EvmResult<u64> {
        log::info!("START of verify()");

        handle.record_log_costs_manual(3, 32)?;

        let query_id = query.id();

        // Build call with origin.
        {
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            log::info!("BEFORE submit_proof() call");

            let result = RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_prover::Call::<Runtime>::submit_proof {
                    proof: proof.clone().into(),
                    query,
                },
                0,
            );

            log::info!("AFTER submit_proof(), result={:?}", result);

            // Instead of erroring out, we propagate status codes to the prover smart contract
            // and let it deal with them.
            // 0. Success, 1. ProofInvalid, 2. LayoutMismatch, 3. OutOfBounds
            match result {
                Ok(_) => {
                    log3(
                        handle.context().address,
                        SELECTOR_LOG_PROOF_SUBMITTED,
                        handle.context().caller,
                        query_id,
                        solidity::encode_event_data(proof),
                    )
                    .record(handle)?;

                    Ok(0)
                }
                Err(e) => match e {
                    TryDispatchError::Evm(_) => Ok(5),
                    TryDispatchError::Substrate(dispatch_error) => match dispatch_error {
                        DispatchError::Module(module_error) => {
                            let error = module_error.error;
                            match error {
                                [0, 0, 0, 0] => {
                                    error!("Invalid proof submitted: {:?}", e);
                                    Ok(1)
                                }
                                [11, 0, 0, 0] => {
                                    error!("Query layout mismatch: {:?}", e);
                                    Ok(2)
                                }
                                [10, 0, 0, 0] => {
                                    error!("Query out of bounds: {:?}", e);
                                    Ok(3)
                                }
                                _ => {
                                    error!("Failed to dispatch submit_proof: {:?}", e);
                                    Ok(1)
                                }
                            }
                        }
                        _ => {
                            error!("Failed to dispatch submit_proof: {:?}", e);
                            Ok(3)
                        }
                    },
                },
            }
        }
    }

    #[precompile::public("get_result_segments(bytes32)")]
    fn get_result_segments(
        _handle: &mut impl PrecompileHandle,
        query_id: H256,
    ) -> EvmResult<Vec<ResultSegment>> {
        let result_segments: Option<_> = ResultSegmentsById::<Runtime>::get(query_id);
        if let Some(segments) = result_segments {
            Ok(Vec::from(segments))
        } else {
            let err = format!("Result segments not found for query: {:?}", query_id);
            error!("{}", err);
            return Err(PrecompileFailure::Error {
                exit_status: fp_evm::ExitError::Other(sp_std::borrow::Cow::Owned(err)),
            });
        }
    }
}
