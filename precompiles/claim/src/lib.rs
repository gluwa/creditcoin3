#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::{Dispatchable, Hash},
};
use pallet_evm::AddressMapping;
use pallet_prover::{types::Prover, ChainPriceConfiguration};
use precompile_utils::prelude::*;
use prover_primitives::claim::{Claim, ClaimKind};
use sp_core::{H160, H256};
use sp_std::vec::Vec;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Solidity selector of the ClaimSubmitted log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_CLAIM_SUBMITTED: [u8; 32] = keccak256!("ClaimSubmitted(bytes32)");

/// Solidity selector of the ProofSubmitted log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_PROOF_SUBMITTED: [u8; 32] = keccak256!("ProofSubmitted(bytes32)");

/// Precompile exposing a pallet_balance as an ERC20.
/// The precompile uses an additional storage to store approvals.
pub struct ClaimPrecompile<Runtime>(PhantomData<Runtime>);

#[derive(Debug, Clone, PartialEq, Eq, precompile_utils::solidity::Codec)]
pub struct ChainPriceConfig {
    chain_id: u64,
    price: u64,
}

impl Into<ChainPriceConfiguration> for ChainPriceConfig {
    fn into(self) -> ChainPriceConfiguration {
        ChainPriceConfiguration {
            chain_id: self.chain_id,
            price: self.price,
        }
    }
}

#[precompile_utils::precompile]
impl<Runtime> ClaimPrecompile<Runtime>
where
    Runtime: pallet_prover::Config + pallet_evm::Config,
    Runtime::Hash: Into<H256>,
    H256: Into<Runtime::Hash>,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_prover::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]>,
    <Runtime as pallet_prover::Config>::Address: From<H160>,
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
    ) -> EvmResult<H256> {
        handle.record_log_costs_manual(3, 32)?;

        // TODO: handle the case where it's both tx & rx
        let kind = if is_tx {
            ClaimKind::Tx
        } else if is_rx {
            ClaimKind::Rx
        } else {
            return Err(revert("Must be either Tx or Rx"));
        };

        let claim = Claim {
            chain_id,
            block_number,
            tx_index,
            from: <Runtime as pallet_prover::Config>::Address::from(from.into()),
            to: <Runtime as pallet_prover::Config>::Address::from(to.into()),
            kind,
        };

        // Hash the claim
        let claim_hash: H256 = <Runtime as pallet_prover::Config>::Hashing::hash_of(&claim).into();

        // Build call with origin.
        {
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            log::debug!("submitting claim with hash: {:?}", claim_hash);
            RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_prover::Call::<Runtime>::submit_claim { claim },
            )?;
        }

        log3(
            handle.context().address,
            SELECTOR_LOG_CLAIM_SUBMITTED,
            handle.context().caller,
            claim_hash,
            solidity::encode_event_data((chain_id, block_number, tx_index, from, to, is_tx, is_rx)),
        )
        .record(handle)?;

        Ok(claim_hash)
    }

    #[precompile::public("register_prover(uint8[])")]
    fn register_prover(handle: &mut impl PrecompileHandle, nickname: Vec<u8>) -> EvmResult<bool> {
        handle.record_log_costs_manual(3, 32)?;

        // Build call with origin.
        {
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            log::debug!("registering prover with nickname: {:?}", nickname);
            RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_prover::Call::<Runtime>::register_prover {
                    prover: Prover { nickname },
                },
            )?;
        }

        Ok(true)
    }

    #[precompile::public("set_chain_price_config((uint64,uint64)[])")]
    fn set_chain_price_config(
        handle: &mut impl PrecompileHandle,
        chain_price_configs: Vec<ChainPriceConfig>,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(3, 32)?;

        // Build call with origin.
        {
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            log::debug!(
                "setting chain price configurations: {:?}",
                chain_price_configs
            );

            let parsed_chain_price_configs: Vec<ChainPriceConfiguration> = chain_price_configs
                .into_iter()
                .map(|config| config.into())
                .collect();

            RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_prover::Call::<Runtime>::set_chain_price_config {
                    chain_price_configs: parsed_chain_price_configs,
                },
            )?;
        }

        Ok(true)
    }

    #[precompile::public("submit_proof(bytes32,uint8[])")]
    fn submit_proof(
        handle: &mut impl PrecompileHandle,
        claim_hash: H256,
        proof: Vec<u8>,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(3, 32)?;

        // Build call with origin.
        {
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            log::debug!(
                "submitting claim with hash: {:?}, for origin: {:?}",
                claim_hash,
                origin
            );
            RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_prover::Call::<Runtime>::submit_proof {
                    claim_hash: claim_hash.into(),
                    proof,
                },
            )?;
        }

        log3(
            handle.context().address,
            SELECTOR_LOG_PROOF_SUBMITTED,
            handle.context().caller,
            claim_hash,
            solidity::encode_event_data(claim_hash),
        )
        .record(handle)?;

        Ok(true)
    }
}
