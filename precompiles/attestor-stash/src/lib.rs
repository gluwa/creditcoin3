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
//! `withdraw_unbonded`, `bond_extra`, `unbond_surplus`). Operator-gated calls
//! (anything behind `OperatorsOrigin`) and attestor-authored calls (`attest`)
//! are intentionally *not* exposed here.
//!
//! `bond_extra` / `unbond_surplus` must be routable from here, not only as
//! Substrate extrinsics: an EVM-space stash is `blake2_256("evm:" || address)`,
//! a hash with no signing key, so an extrinsic-only surplus path is unreachable
//! for it. `unbond_surplus` is the *only* release path for bond above the
//! stash's aggregate requirement — `unregister_attestor` releases at most the
//! *current* `MinBondRequirement` per attestor and `withdraw_unbonded` only
//! reaps once `active` is below the existential deposit — so without it a
//! governance *decrease* of `MinBondRequirement` would strand the difference in
//! the bond pool permanently for every EVM-space stash.
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
use sp_core::{H256, U256};
use sp_runtime::{traits::UniqueSaturatedInto, Saturating};
use sp_staking::StakingInterface;
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
///
/// `withdrawable` is the sum of all unlocking chunks whose era has already
/// elapsed (i.e. chunks that `withdrawUnbonded` would actually return). It
/// lets callers distinguish between "funds are unlocking but not yet ready"
/// and "funds are ready to withdraw" without a second call.
///
/// `stash` is the hashed `AccountId32` the ledger is keyed by. It lets callers
/// (and tests) confirm which account a ledger actually belongs to, rather than
/// trusting that two lookups returning identical balances refer to the same
/// stash. It is `H256::zero()` when `exists == false`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct LedgerInfo {
    pub exists: bool,
    pub stash: H256,
    pub total_staked: u128,
    pub active: u128,
    pub unlocking_chunks: u32,
    pub withdrawable: u128,
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

/// `BondExtraAdded(address indexed stash, uint256 amount)`
pub const SELECTOR_LOG_BOND_EXTRA_ADDED: [u8; 32] = keccak256!("BondExtraAdded(address,uint256)");

/// `SurplusUnbonded(address indexed stash, uint256 amount)`
pub const SELECTOR_LOG_SURPLUS_UNBONDED: [u8; 32] = keccak256!("SurplusUnbonded(address,uint256)");

/// Precompile exposing stash-facing calls of `pallet_attestation`.
pub struct AttestorStashPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
impl<Runtime> AttestorStashPrecompile<Runtime>
where
    Runtime: pallet_attestation::Config + pallet_evm::Config,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_attestation::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]> + Into<[u8; 32]>,
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
                    attestor_primitives::AttestorStatus::Leaving => 3,
                };
                let stash: H256 = H256::from(Into::<[u8; 32]>::into(attestor.stash));
                let has_bls_key = attestor.bls_public_key.is_some();
                Ok(AttestorInfo {
                    exists: true,
                    status: status_u8,
                    stash,
                    has_bls_key,
                })
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
    fn get_attestors_count(handle: &mut impl PrecompileHandle, chain_key: u64) -> EvmResult<u32> {
        let count = AttestorsCount::<Runtime>::get(chain_key as ChainKey);
        use parity_scale_codec::Encode;
        handle.record_db_read::<Runtime>(count.encoded_size())?;
        Ok(count)
    }

    /// Returns ledger info for a given stash account.
    ///
    /// `stash` must be the **hashed** `AccountId32` produced by `AddressMapping` from the EVM
    /// address — not the raw 20-byte EVM address zero-padded to 32 bytes. EVM consumers
    /// emitting events tied to their own `msg.sender` should prefer
    /// [`Self::get_ledger_by_address`] or [`Self::get_caller_ledger`], which apply
    /// `AddressMapping` internally and avoid the silently-empty-ledger foot-gun.
    #[precompile::public("getLedger(bytes32)")]
    #[precompile::view]
    fn get_ledger(handle: &mut impl PrecompileHandle, stash: H256) -> EvmResult<LedgerInfo> {
        let account = Runtime::AccountId::from(stash.0);
        Self::get_ledger_for_account(handle, account)
    }

    /// Same as [`Self::get_ledger`] but takes the raw EVM `address` and applies the runtime's
    /// `AddressMapping` internally. This is what EVM-side consumers usually want: events and
    /// state-changing calls in this precompile already use the EVM `address` as the caller
    /// identifier, so symmetric reads with the same key avoid the historical foot-gun where
    /// users converted the emitted address to `bytes32` and got an empty ledger back.
    #[precompile::public("getLedgerByAddress(address)")]
    #[precompile::view]
    fn get_ledger_by_address(
        handle: &mut impl PrecompileHandle,
        addr: Address,
    ) -> EvmResult<LedgerInfo> {
        let account = Runtime::AddressMapping::into_account_id(addr.into());
        Self::get_ledger_for_account(handle, account)
    }

    /// Same as [`Self::get_ledger_by_address`] but uses the EVM caller (`msg.sender`).
    /// Convenience entry for self-lookups; saves the caller from passing their own address.
    ///
    /// Implemented as a delegation to [`Self::get_ledger_by_address`] so the
    /// `AddressMapping::into_account_id` step lives in exactly one place — keeps the two
    /// helpers from drifting if the mapping behavior ever changes.
    #[precompile::public("getCallerLedger()")]
    #[precompile::view]
    fn get_caller_ledger(handle: &mut impl PrecompileHandle) -> EvmResult<LedgerInfo> {
        let caller_evm = handle.context().caller;
        Self::get_ledger_by_address(handle, Address(caller_evm))
    }

    /// Shared ledger lookup body. Kept private so all three public entries (`getLedger`,
    /// `getLedgerByAddress`, `getCallerLedger`) charge the same gas and return identical
    /// results.
    fn get_ledger_for_account(
        handle: &mut impl PrecompileHandle,
        account: Runtime::AccountId,
    ) -> EvmResult<LedgerInfo> {
        let stash = H256::from(Into::<[u8; 32]>::into(account.clone()));
        match Ledger::<Runtime>::get(&account) {
            None => {
                handle.record_db_read::<Runtime>(0)?;
                Ok(LedgerInfo::default())
            }
            Some(ledger) => {
                use parity_scale_codec::Encode;
                handle.record_db_read::<Runtime>(ledger.encoded_size())?;
                // Sum unlocking chunks whose unbonding era has already passed.
                let current_era = <Runtime as pallet_attestation::Config>::Staking::current_era();
                let withdrawable = ledger
                    .unlocking
                    .iter()
                    .filter(|chunk| chunk.era <= current_era)
                    .fold(
                        <pallet_attestation::BalanceOf<Runtime>>::default(),
                        |acc, chunk| acc.saturating_add(chunk.value),
                    );
                Ok(LedgerInfo {
                    exists: true,
                    stash,
                    total_staked: ledger.total_staked.unique_saturated_into(),
                    active: ledger.active.unique_saturated_into(),
                    unlocking_chunks: ledger.unlocking.len() as u32,
                    withdrawable: withdrawable.unique_saturated_into(),
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

    /// Top up the caller's stash bond by `amount` of attest coin, without registering another
    /// attestor. Moves the amount from the stash's liquid `pallet-assets` balance into the bond
    /// pool and credits `total_staked` / `active`.
    ///
    /// Mirrors `pallet_attestation::Call::bond_extra`. The pallet rejects a stash with no ledger
    /// (`NotStash` — a stash with no registered attestor exits via `withdrawUnbonded` instead) and
    /// an amount above the stash's liquid balance (`InsufficientBalance`); both surface here as a
    /// revert.
    #[precompile::public("bondExtra(uint256)")]
    fn bond_extra(handle: &mut impl PrecompileHandle, amount: U256) -> EvmResult<bool> {
        handle.record_log_costs_manual(2, 32)?;

        let value = Self::u256_to_balance(amount).in_field("amount")?;
        let caller_evm = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller_evm);

        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            pallet_attestation::Call::<Runtime>::bond_extra { amount: value },
            0,
        )?;

        log2(
            handle.context().address,
            SELECTOR_LOG_BOND_EXTRA_ADDED,
            H256::from(caller_evm),
            solidity::encode_event_data(amount),
        )
        .record(handle)?;

        Ok(true)
    }

    /// Move `amount` of surplus bond into an unlocking chunk so `withdrawUnbonded` can return it.
    ///
    /// Mirrors `pallet_attestation::Call::unbond_surplus`. The inverse of `bondExtra`, and the only
    /// way to release bond that sits above the stash's aggregate requirement — reachable, for
    /// example, after governance *lowers* a chain's `MinBondRequirement` below what the stash bonded
    /// at registration. `active` may only be reduced down to the sum of `MinBondRequirement` across
    /// the stash's still-registered attestors, never below (`InsufficientRemainingBond` otherwise);
    /// with no attestors registered that sum is zero, so the whole remaining bond is releasable and
    /// the stash can then be reaped by `withdrawUnbonded`.
    #[precompile::public("unbondSurplus(uint256)")]
    fn unbond_surplus(handle: &mut impl PrecompileHandle, amount: U256) -> EvmResult<bool> {
        handle.record_log_costs_manual(2, 32)?;

        let value = Self::u256_to_balance(amount).in_field("amount")?;
        let caller_evm = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller_evm);

        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            pallet_attestation::Call::<Runtime>::unbond_surplus { amount: value },
            0,
        )?;

        log2(
            handle.context().address,
            SELECTOR_LOG_SURPLUS_UNBONDED,
            H256::from(caller_evm),
            solidity::encode_event_data(amount),
        )
        .record(handle)?;

        Ok(true)
    }

    /// `uint256` → the attestation pallet's balance type.
    ///
    /// Reverts rather than truncating. `CurrencyBalance` is 128-bit, so a wider `uint256` argument
    /// has no faithful representation; silently wrapping it would bond or release an amount other
    /// than the one the caller signed for. `uint256` is the parameter type rather than `uint128`
    /// because callers reach these two entries straight from the attest-coin precompile's
    /// `deposit(uint256)` / `withdraw(uint256)` and from ERC-20 balances, all of which are
    /// `uint256`; narrowing at the ABI boundary would just move the truncation into the caller.
    fn u256_to_balance(value: U256) -> MayRevert<pallet_attestation::BalanceOf<Runtime>> {
        let raw: u128 = value
            .try_into()
            .map_err(|_| RevertReason::value_is_too_large("uint128"))?;
        Ok(pallet_attestation::BalanceOf::<Runtime>::from(raw))
    }
}
