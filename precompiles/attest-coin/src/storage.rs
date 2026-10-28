//! Meter native storage separately from the ERC-20 subcalls' EVM storage.

use fp_evm::{ExitError, PrecompileHandle};
use frame_system::{pallet_prelude::BlockNumberFor, AccountInfo};
use parity_scale_codec::MaxEncodedLen;
use precompile_utils::prelude::{PrecompileHandleExt, RuntimeHelper};
use sp_runtime::traits::Zero;

// FRAME's MaxEncodedLen proof estimate adds this trie overhead per storage entry. Include the
// full key as well as the value; repeated accesses may overestimate a shared proof, never omit it.
const TRIE_PROOF_OVERHEAD: usize = 2_475;

pub(crate) fn record_read<R: pallet_evm::Config>(
    handle: &mut impl PrecompileHandle,
    key_len: usize,
    value_len: usize,
) -> Result<(), ExitError> {
    handle.record_db_read::<R>(TRIE_PROOF_OVERHEAD + key_len + value_len)
}

/// Reserve a reward-map mutation before changing storage or calling the token. Covers the
/// existence probe, the mutation, and a possible undo after a failed ERC-20 subcall. The proof
/// is charged once per key; growth is charged only when the entry does not already exist.
pub(crate) fn record_mutation<R: pallet_evm::Config>(
    handle: &mut impl PrecompileHandle,
    key: &[u8],
    value_len: usize,
) -> Result<(), ExitError> {
    record_read::<R>(handle, key.len(), value_len)?;
    let growth = if sp_io::storage::exists(key) {
        0
    } else {
        (key.len() + value_len) as u64
    };
    handle.record_external_cost(None, None, Some(growth))?;
    handle.record_cost(
        RuntimeHelper::<R>::db_read_gas_cost()
            .saturating_add(RuntimeHelper::<R>::db_write_gas_cost())
            .saturating_mul(2),
    )
}

pub(crate) fn record_genesis_hash_read<R: pallet_evm::Config>(
    handle: &mut impl PrecompileHandle,
) -> Result<(), ExitError> {
    let key = frame_system::BlockHash::<R>::hashed_key_for(BlockNumberFor::<R>::zero());
    record_read::<R>(handle, key.len(), R::Hash::max_encoded_len())
}

/// Chain metadata contains variable-length strings. Read just its stored length rather than
/// decoding/allocating the value, and charge that full length before proceeding with the claim.
pub(crate) fn supported_chain_exists<R: pallet_evm::Config + pallet_supported_chains::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: u64,
) -> Result<bool, ExitError> {
    let key = pallet_supported_chains::SupportedChains::<R>::hashed_key_for(chain_key);
    record_read::<R>(handle, key.len(), 0)?;
    let len = sp_io::storage::read(&key, &mut [], 0);
    handle.record_external_cost(None, Some(u64::from(len.unwrap_or(0))), None)?;
    Ok(len.is_some())
}

pub(crate) fn record_asset_account_read<R: pallet_evm::Config + pallet_assets::Config>(
    handle: &mut impl PrecompileHandle,
    id: &R::AssetId,
    beneficiary: &R::AccountId,
) -> Result<(), ExitError> {
    let key = pallet_assets::Account::<R>::hashed_key_for(id, beneficiary);
    record_read::<R>(
        handle,
        key.len(),
        pallet_assets::AssetAccountOf::<R, ()>::max_encoded_len(),
    )
}

/// Asset-account creation/removal can change a consumer/sufficient reference on System::Account.
pub(crate) fn record_system_account_change<R: pallet_evm::Config>(
    handle: &mut impl PrecompileHandle,
    beneficiary: &R::AccountId,
) -> Result<usize, ExitError> {
    let key = frame_system::Account::<R>::hashed_key_for(beneficiary);
    let value_size = AccountInfo::<R::Nonce, R::AccountData>::max_encoded_len();
    record_read::<R>(handle, key.len(), value_size)?;
    handle.record_cost(RuntimeHelper::<R>::db_write_gas_cost())?;
    Ok(key.len() + value_size)
}

/// A new asset account adds its full key/value. Creating it also updates the beneficiary's
/// System account (consumer/sufficient reference); sufficient assets can create that account.
/// Existing asset accounts only change fixed-width balances and do not grow storage.
pub(crate) fn mint_storage_growth<R: pallet_evm::Config + pallet_assets::Config>(
    handle: &mut impl PrecompileHandle,
    id: &R::AssetId,
    beneficiary: &R::AccountId,
) -> Result<u64, ExitError> {
    record_asset_account_read::<R>(handle, id, beneficiary)?;
    if pallet_assets::Account::<R>::contains_key(id, beneficiary) {
        return Ok(0);
    }
    let asset_key = pallet_assets::Account::<R>::hashed_key_for(id, beneficiary);
    let mut growth = asset_key.len() + pallet_assets::AssetAccountOf::<R, ()>::max_encoded_len();
    let system_account_size = record_system_account_change::<R>(handle, beneficiary)?;
    if !frame_system::Account::<R>::contains_key(beneficiary) {
        growth += system_account_size;
    }
    Ok(growth as u64)
}
