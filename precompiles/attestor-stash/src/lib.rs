#![cfg_attr(not(feature = "std"), no_std)]

//! Attestor Stash Precompile
//!
//! Exposes stash-facing dispatchables of `pallet-attestation` through an EVM
//! precompile. This lets a stash account that lives in the EVM address space
//! (i.e. whose `H160` maps to a Substrate `AccountId`) interact with the
//! attestation pallet without having to construct a raw Substrate extrinsic.
//!
//! The precompile only exposes the subset of pallet calls that are authored
//! by a stash (`register_attestor`, `unregister_attestor`, `chill`,
//! `withdraw_unbonded`). Operator-gated calls (anything behind
//! `OperatorsOrigin`) and attestor-authored calls (`attest`) are
//! intentionally *not* exposed here.
//!
//! The precompile is accessible at address `0x0FD4` (4052 in decimal) in the
//! Creditcoin 3 runtime.

use core::marker::PhantomData;
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use pallet_attestation::{ActiveAttestors, Attestors, AttestorsCount, Ledger, MinBondRequirement};
use pallet_evm::AddressMapping;
use precompile_utils::{
    evm::logs::{log2, log4},
    keccak256,
    prelude::*,
    solidity::Codec,
};
use sp_core::H256;
use sp_runtime::traits::UniqueSaturatedInto;
use sp_std::vec::Vec;

use attestor_primitives::ChainKey;

/// Return type for `getAttestor`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct AttestorInfo {
    pub exists: bool,
    pub status: u8,
    pub stash: H256,
    pub has_bls_key: bool,
}

/// Return type for `getLedger`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct LedgerInfo {
    pub exists: bool,
    pub total_staked: u128,
    pub active: u128,
    pub unlocking_chunks: u32,
}

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// `AttestorRegistered(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash)`
pub const SELECTOR_LOG_ATTESTOR_REGISTERED: [u8; 32] =
    keccak256!("AttestorRegistered(uint64,bytes32,address)");

/// `AttestorUnregistered(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash)`
pub const SELECTOR_LOG_ATTESTOR_UNREGISTERED: [u8; 32] =
    keccak256!("AttestorUnregistered(uint64,bytes32,address)");

/// `AttestorChilled(uint64 indexed chainKey, bytes32 indexed attestorId, address indexed stash)`
pub const SELECTOR_LOG_ATTESTOR_CHILLED: [u8; 32] =
    keccak256!("AttestorChilled(uint64,bytes32,address)");

/// `UnbondedWithdrawn(address indexed stash)`
pub const SELECTOR_LOG_UNBONDED_WITHDRAWN: [u8; 32] = keccak256!("UnbondedWithdrawn(address)");

/// Precompile exposing stash-facing calls of `pallet_attestation`.
pub struct AttestorStashPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
impl<Runtime> AttestorStashPrecompile<Runtime>
where
    Runtime: pallet_attestation::Config + pallet_evm::Config,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_attestation::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]> + Into<H256>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    /// Register a new attestor under the caller's stash for the given chain.
    ///
    /// Mirrors `pallet_attestation::Call::register_attestor`.
    #[precompile::public("registerAttestor(uint64,bytes32)")]
    fn register_attestor(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        attestor_id: H256,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(4, 0)?;

        let caller_evm = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller_evm);
        let attestor_account = Runtime::AccountId::from(attestor_id.0);

        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            pallet_attestation::Call::<Runtime>::register_attestor {
                chain_key: chain_key as ChainKey,
                attestor_id: attestor_account,
            },
            0,
        )?;

        log4(
            handle.context().address,
            SELECTOR_LOG_ATTESTOR_REGISTERED,
            H256::from_low_u64_be(chain_key),
            attestor_id,
            H256::from(caller_evm),
            Vec::<u8>::new(),
        )
        .record(handle)?;

        Ok(true)
    }

    /// Unregister an attestor previously registered by the caller's stash for
    /// the given chain.
    ///
    /// Mirrors `pallet_attestation::Call::unregister_attestor`.
    #[precompile::public("unregisterAttestor(uint64,bytes32)")]
    fn unregister_attestor(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        attestor_id: H256,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(4, 0)?;

        let caller_evm = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller_evm);
        let attestor_account = Runtime::AccountId::from(attestor_id.0);

        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            pallet_attestation::Call::<Runtime>::unregister_attestor {
                chain_key: chain_key as ChainKey,
                attestor_id: attestor_account,
            },
            0,
        )?;

        log4(
            handle.context().address,
            SELECTOR_LOG_ATTESTOR_UNREGISTERED,
            H256::from_low_u64_be(chain_key),
            attestor_id,
            H256::from(caller_evm),
            Vec::<u8>::new(),
        )
        .record(handle)?;

        Ok(true)
    }

    /// Chill an attestor registered by the caller's stash for the given chain.
    ///
    /// `pallet_attestation::chill` is authored by the stash (it checks
    /// `attestor.stash == who`), so it is exposed here rather than in an
    /// attestor-facing surface.
    ///
    /// Mirrors `pallet_attestation::Call::chill`.
    #[precompile::public("chill(uint64,bytes32)")]
    fn chill(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        attestor_id: H256,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(4, 0)?;

        let caller_evm = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller_evm);
        let attestor_account = Runtime::AccountId::from(attestor_id.0);

        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            pallet_attestation::Call::<Runtime>::chill {
                chain_key: chain_key as ChainKey,
                attestor_id: attestor_account,
            },
            0,
        )?;

        log4(
            handle.context().address,
            SELECTOR_LOG_ATTESTOR_CHILLED,
            H256::from_low_u64_be(chain_key),
            attestor_id,
            H256::from(caller_evm),
            Vec::<u8>::new(),
        )
        .record(handle)?;

        Ok(true)
    }

    /// Returns attestor info for a given chain key and attestor id.
    #[precompile::public("getAttestor(uint64,bytes32)")]
    #[precompile::view]
    fn get_attestor(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        attestor_id: H256,
    ) -> EvmResult<AttestorInfo> {
        let account = Runtime::AccountId::from(attestor_id.0);
        match Attestors::<Runtime>::get(chain_key as ChainKey, &account) {
            None => {
                handle.record_db_read::<Runtime>(0)?;
                Ok(AttestorInfo::default())
            }
            Some(attestor) => {
                use parity_scale_codec::Encode;
                handle.record_db_read::<Runtime>(attestor.encoded_size())?;
                let status_u8: u8 = match attestor.status {
                    attestor_primitives::AttestorStatus::Active => 0,
                    attestor_primitives::AttestorStatus::Idle => 1,
                    attestor_primitives::AttestorStatus::Waiting => 2,
                };
                let stash: H256 = attestor.stash.into();
                let has_bls_key = attestor.bls_public_key.is_some();
                Ok(AttestorInfo { exists: true, status: status_u8, stash, has_bls_key })
            }
        }
    }

    /// Returns true if the attestor is in the active set for the given chain.
    #[precompile::public("isActiveAttestor(uint64,bytes32)")]
    #[precompile::view]
    fn is_active_attestor(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
        attestor_id: H256,
    ) -> EvmResult<bool> {
        let active = ActiveAttestors::<Runtime>::get(chain_key as ChainKey);
        use parity_scale_codec::Encode;
        handle.record_db_read::<Runtime>(active.encoded_size())?;
        let account = Runtime::AccountId::from(attestor_id.0);
        Ok(active.contains(&account))
    }

    /// Returns the number of registered attestors for the given chain.
    #[precompile::public("getAttestorsCount(uint64)")]
    #[precompile::view]
    fn get_attestors_count(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
    ) -> EvmResult<u32> {
        let count = AttestorsCount::<Runtime>::get(chain_key as ChainKey);
        use parity_scale_codec::Encode;
        handle.record_db_read::<Runtime>(count.encoded_size())?;
        Ok(count)
    }

    /// Returns ledger info for a given stash account.
    #[precompile::public("getLedger(bytes32)")]
    #[precompile::view]
    fn get_ledger(
        handle: &mut impl PrecompileHandle,
        stash: H256,
    ) -> EvmResult<LedgerInfo> {
        let account = Runtime::AccountId::from(stash.0);
        match Ledger::<Runtime>::get(&account) {
            None => {
                handle.record_db_read::<Runtime>(0)?;
                Ok(LedgerInfo::default())
            }
            Some(ledger) => {
                use parity_scale_codec::Encode;
                handle.record_db_read::<Runtime>(ledger.encoded_size())?;
                Ok(LedgerInfo {
                    exists: true,
                    total_staked: ledger.total_staked.unique_saturated_into(),
                    active: ledger.active.unique_saturated_into(),
                    unlocking_chunks: ledger.unlocking.len() as u32,
                })
            }
        }
    }

    /// Returns the minimum bond requirement for the given chain.
    #[precompile::public("getMinBondRequirement(uint64)")]
    #[precompile::view]
    fn get_min_bond_requirement(
        handle: &mut impl PrecompileHandle,
        chain_key: u64,
    ) -> EvmResult<u128> {
        let min_bond = MinBondRequirement::<Runtime>::get(chain_key as ChainKey);
        use parity_scale_codec::Encode;
        handle.record_db_read::<Runtime>(min_bond.encoded_size())?;
        Ok(min_bond.unique_saturated_into())
    }

    /// Withdraw any fully-unbonded funds for the caller's stash.
    ///
    /// Mirrors `pallet_attestation::Call::withdraw_unbonded`.
    #[precompile::public("withdrawUnbonded()")]
    fn withdraw_unbonded(handle: &mut impl PrecompileHandle) -> EvmResult<bool> {
        handle.record_log_costs_manual(2, 0)?;

        let caller_evm = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller_evm);

        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            pallet_attestation::Call::<Runtime>::withdraw_unbonded {},
            0,
        )?;

        log2(
            handle.context().address,
            SELECTOR_LOG_UNBONDED_WITHDRAWN,
            H256::from(caller_evm),
            Vec::<u8>::new(),
        )
        .record(handle)?;

        Ok(true)
    }
}
