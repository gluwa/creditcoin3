#![cfg_attr(not(feature = "std"), no_std)]

use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::{Dispatchable, StaticLookup},
};
use frame_system::AccountInfo;
use pallet_evm::AddressMapping;
use parity_scale_codec::MaxEncodedLen;
use precompile_utils::prelude::*;
use sp_core::{H256, U256};
use sp_std::{
    convert::{TryFrom, TryInto},
    marker::PhantomData,
};

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Solidity selector of the Transfer log, which is the Keccak of the Log signature.
pub const SELECTOR_LOG_TRANSFER: [u8; 32] = keccak256!("Transfer(address,bytes32,uint256)");

/// Length of the two `twox_128` hashes (pallet and item) that prefix every storage key.
const STORAGE_PREFIX_LEN: usize = 32;
/// Length of the `Blake2_128Concat` hash ahead of the account id in a `System::Account` key.
const ACCOUNT_KEY_HASH_LEN: usize = 16;

/// Alias for the Balance type for the provided Runtime and Instance.
pub type BalanceOf<Runtime, Instance = ()> =
    <Runtime as pallet_balances::Config<Instance>>::Balance;

/// Precompile exposing a pallet_balance as an ERC20.
/// The precompile uses an additional storage to store approvals.
pub struct SubstrateTransferPrecompile<Runtime, Instance: 'static = ()>(
    PhantomData<(Runtime, Instance)>,
);

#[precompile_utils::precompile]
impl<Runtime, Instance> SubstrateTransferPrecompile<Runtime, Instance>
where
    Runtime: pallet_balances::Config<Instance> + pallet_evm::Config,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_balances::Call<Runtime, Instance>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    BalanceOf<Runtime, Instance>: TryFrom<U256> + Into<U256>,
    Instance: 'static,
    Runtime::AccountId: From<[u8; 32]>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    #[precompile::public("transfer_substrate(bytes32,uint256)")]
    fn transfer_substrate(
        handle: &mut impl PrecompileHandle,
        destination: H256,
        amount: U256,
    ) -> EvmResult<bool> {
        handle.record_log_costs_manual(3, 32)?;

        // Build call with origin.
        {
            log::debug!("bytes: {destination:?}");
            let origin = Runtime::AddressMapping::into_account_id(handle.context().caller);

            let to = Runtime::AccountId::from(destination.0);

            let amount = Self::u256_to_amount(amount).in_field("value")?;

            // Only a transfer to a fresh destination adds persistent state.
            let storage_growth = if frame_system::Pallet::<Runtime>::account_exists(&to) {
                0
            } else {
                Self::new_account_storage_growth()
            };

            // Dispatch call (if enough gas).
            RuntimeHelper::<Runtime>::try_dispatch(
                handle,
                Some(origin).into(),
                pallet_balances::Call::<Runtime, Instance>::transfer_allow_death {
                    dest: Runtime::Lookup::unlookup(to),
                    value: amount,
                },
                storage_growth,
            )?;
        }

        log3(
            handle.context().address,
            SELECTOR_LOG_TRANSFER,
            handle.context().caller,
            destination,
            solidity::encode_event_data(amount),
        )
        .record(handle)?;

        Ok(true)
    }

    /// Bytes a transfer adds to storage by creating the destination's `System::Account` entry:
    /// the full storage key plus the largest encoded `AccountInfo`.
    pub fn new_account_storage_growth() -> u64 {
        let key = STORAGE_PREFIX_LEN
            + ACCOUNT_KEY_HASH_LEN
            + <Runtime::AccountId as MaxEncodedLen>::max_encoded_len();
        let value = AccountInfo::<Runtime::Nonce, Runtime::AccountData>::max_encoded_len();
        (key + value) as u64
    }

    fn u256_to_amount(value: U256) -> MayRevert<BalanceOf<Runtime, Instance>> {
        value
            .try_into()
            .map_err(|_| RevertReason::value_is_too_large("balance type").into())
    }
}
