#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use log::error;
use pallet_evm::AddressMapping;
use pallet_prover::StarkProgramMetadata;
use pallet_prover_primitives::{Query, ResultSegment, VerifierExitStatus};
use precompile_utils::prelude::*;
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
    ) -> EvmResult<(u64, Vec<ResultSegment>)> {
        handle.record_log_costs_manual(3, 32)?;

        // Pre eliminary check
        if proof.as_bytes().is_empty() {
            error!("Invalid proof submitted for query: {:?}", query);
            return Ok((2, Vec::new()));
        }

        let query_id = query.id();

        {
            let metadata = StarkProgramMetadata::<Runtime>::iter().collect::<Vec<_>>();
            if metadata.is_empty() {
                error!("Stark program metadata not set for query: {:?}", query);
                return Ok((4, Vec::new()));
            }
            let (result_status, result_segments) =
                proof_verifier::host_api::verify_proof(proof.clone().into(), query, metadata);

            // Instead of erroring out, we propagate status codes to the prover smart contract
            // and let it deal with them. 1 indicating `LayoutMismatch`, 2 - `ProofInvalid`, etc.
            match result_status {
                0 => {
                    // Build call with origin to emit event
                    let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);
                    let result = RuntimeHelper::<Runtime>::try_dispatch(
                        handle,
                        Some(origin).into(),
                        pallet_prover::Call::<Runtime>::post_query_result {
                            query_id,
                            verifier_exit_status: VerifierExitStatus::Success,
                        },
                        0,
                    );
                    if result.is_err() {
                        error!("post_query_result failed")
                    }
                    Ok((0, result_segments))
                }
                12 => {
                    error!("Query out of bounds: {:?}", query_id);
                    Ok((3, result_segments))
                }
                13 => {
                    error!("Query layout mismatch: {:?}", query_id);
                    Ok((1, result_segments))
                }
                _ => {
                    error!("Unknown proving error for query: {:?}", query_id);
                    Ok((4, result_segments))
                }
            }
        }
    }
}
