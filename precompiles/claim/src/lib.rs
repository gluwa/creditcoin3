#![cfg_attr(not(feature = "std"), no_std)]

use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use pallet_evm::AddressMapping;
use precompile_utils::prelude::*;
use prover_primitives::claim::{Claim, ClaimKind};
use sp_std::marker::PhantomData;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Solidity selector of the Transfer log, which is the Keccak of the Log signature.
/// ClaimSubmitted(claimID) TODO
pub const SELECTOR_LOG_TRANSFER: [u8; 32] = keccak256!("ClaimSubmitted(uint64)");

/// Precompile exposing a pallet_balance as an ERC20.
/// The precompile uses an additional storage to store approvals.
pub struct ClaimPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
impl<Runtime> ClaimPrecompile<Runtime>
where
    Runtime: pallet_prover::Config + pallet_evm::Config,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_prover::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]>,
    <Runtime as pallet_prover::Config>::Address: From<precompile_utils::prelude::Address>,
{
    #[precompile::public("submit_claim(uint64,uint64,uint8,address,address,bool,bool)")]
    fn submit_claim(
        handle: &mut impl PrecompileHandle,
        chain_id: u64,
        block_number: u64,
        tx_index: u8,
        from: Address,
        to: Address,
        is_tx: bool,
        is_rx: bool,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(3, 32)?;

        // Build call with origin.
        {
            log::debug!("claim: {:?}", chain_id);
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            let kind = if is_tx {
                ClaimKind::Tx
            } else if is_rx {
                ClaimKind::Rx
            } else {
                ClaimKind::Tx
            };

            let claim = Claim {
                chain_id,
                block_number,
                tx_index,
                from: from.into(),
                to: to.into(),
                kind,
            };

            RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_prover::Call::<Runtime>::submit_claim { claim },
            )?;
        }

        // log3(
        //     handle.context().address,
        //     SELECTOR_LOG_TRANSFER,
        //     handle.context().caller,
        //     origin,
        //     solidity::encode_event_data(claim),
        // )
        // .record(handle)?;

        Ok(true)
    }
}
