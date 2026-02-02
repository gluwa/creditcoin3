#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use sp_core::{Encode, H256};
use sp_std::vec::Vec;

use attestor_primitives::{ChainId, ChainKey};
use pallet_attestation_poc::{
    AttestationChainGenesisBlockNumber, Attestations, CheckpointBuckets, Checkpoints,
    LastCheckpoint, LastDigest, Pallet as PalletAttestationPoc, CHECKPOINT_BUCKET_SIZE,
};
use pallet_evm::AddressMapping;
use pallet_supported_chains::SupportedChains;
use precompile_utils::{prelude::*, solidity::Codec};

// Gas cost constants
/// Cost of each storage read (matches cold SLOAD) in gas.
pub const GAS_STORAGE_LOOKUP: u64 = 2_600;
/// Per item processed in iteration
pub const GAS_PER_ITERATION_ITEM: u64 = 26;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Precompile exposing source chain info to the evm.
pub struct ChainInfoPrecompile<Runtime>(PhantomData<Runtime>);

const BUCKET_SEARCH_ATTEMPTS: u32 = 5; // Number of attempts to search through checkpoint buckets

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct ChainInfo {
    pub chain_key: ChainKey,
    pub chain_id: ChainId,
    pub chain_name: UnboundedBytes,
    pub chain_encoding: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct ChainInfoResult {
    pub chain: ChainInfo,
    pub exists: bool,
}

impl ChainInfoResult {
    pub fn with_chain(chain: ChainInfo) -> Self {
        Self {
            chain,
            exists: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct HeightResult {
    pub height: u64,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct HeightHashResult {
    pub height: u64,
    pub hash: H256,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct BoundsCheckResult {
    pub parent: u64,
    pub parent_hash: H256,
    pub parent_is_attestation: bool,
    pub child: u64,
    pub child_hash: H256,
    pub child_is_attestation: bool,
    pub is_attested: bool,
}

#[precompile_utils::precompile]
impl<Runtime> ChainInfoPrecompile<Runtime>
where
    Runtime: pallet_supported_chains::Config + pallet_evm::Config + pallet_attestation_poc::Config,
    Runtime::Hash: Into<H256>,
    H256: Into<Runtime::Hash>,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    Runtime::RuntimeCall: From<pallet_supported_chains::Call<Runtime>>,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::AccountId: From<[u8; 32]>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    #[precompile::public("get_supported_chains()")]
    #[precompile::view]
    fn get_supported_chains(handle: &mut impl PrecompileHandle) -> EvmResult<Vec<ChainInfo>> {
        SupportedChains::<Runtime>::iter()
            .map(|(chain_key, sc)| {
                handle.record_db_read::<Runtime>(sc.encoded_size())?;
                let chain = ChainInfo {
                    chain_key,
                    chain_id: sc.chain_id,
                    chain_name: UnboundedBytes::from(sc.chain_name),
                    chain_encoding: sc.chain_encoding as u8,
                };

                Ok(chain)
            })
            .collect::<EvmResult<Vec<ChainInfo>>>()
    }

    #[precompile::public("get_chain_by_key(uint64)")]
    #[precompile::view]
    fn get_chain_by_key(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<ChainInfoResult> {
        if let Some(sc) = SupportedChains::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(sc.encoded_size())?;
            let chain = ChainInfo {
                chain_key,
                chain_id: sc.chain_id,
                chain_name: UnboundedBytes::from(sc.chain_name),
                chain_encoding: sc.chain_encoding as u8,
            };

            Ok(ChainInfoResult::with_chain(chain))
        } else {
            // We want an empty return rather than a revert here
            Ok(ChainInfoResult::default())
        }
    }

    #[precompile::public("get_attestation_genesis_height(uint64)")]
    #[precompile::view]
    fn get_attestation_genesis_height(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<u64> {
        let height = AttestationChainGenesisBlockNumber::<Runtime>::get(chain_key);

        handle.record_db_read::<Runtime>(height.encoded_size())?;

        Ok(height)
    }

    #[precompile::public("get_latest_attestation_height_and_hash(uint64)")]
    #[precompile::view]
    fn get_latest_attestation_height_and_hash(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<HeightHashResult> {
        if let Some(last_digest) = LastDigest::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(last_digest.encoded_size())?;

            Ok(HeightHashResult {
                height: last_digest.0,
                hash: last_digest.1,
                exists: true,
            })
        } else if let Some(last_checkpoint) = LastCheckpoint::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(last_checkpoint.encoded_size())?;

            Ok(HeightHashResult {
                height: last_checkpoint.block_number,
                hash: last_checkpoint.digest,
                exists: true,
            })
        } else {
            Ok(HeightHashResult::default())
        }
    }

    #[precompile::public("get_latest_checkpoint_height_and_hash(uint64)")]
    #[precompile::view]
    fn get_latest_checkpoint_height_and_hash(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<HeightHashResult> {
        if let Some(last_checkpoint) = LastCheckpoint::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(last_checkpoint.encoded_size())?;

            Ok(HeightHashResult {
                height: last_checkpoint.block_number,
                hash: last_checkpoint.digest,
                exists: true,
            })
        } else {
            Ok(HeightHashResult::default())
        }
    }

    #[precompile::public("find_highest_attested_before(uint64,uint64)")]
    #[precompile::view]
    fn find_highest_attested_before(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        target_height: u64,
    ) -> EvmResult<HeightHashResult> {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let maybe_last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        // We first check if the latest checkpoint height is higher or equal to the target height.
        if matches!(maybe_last_checkpoint_height, Some(height) if height >= target_height) {
            // If it is search through the checkpoints to find the highest attested height below the target.
            // We search through the bucket of the block corresponding to target_height - 1 for any checkpoints.
            let mut block_pivot = PalletAttestationPoc::<Runtime>::compute_block_index_for(
                target_height.saturating_sub(1),
            );

            let mut maybe_highest = None;

            // We limit the number of bucket searches to avoid excessive computation.
            for _ in 0..BUCKET_SEARCH_ATTEMPTS {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                let mut items_processed = 0_u64;

                // We search the checkpoint bucket for the highest checkpoint below the target height.
                maybe_highest =
                    CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, block_pivot))
                        .inspect(|_| {
                            items_processed += 1;
                        })
                        .filter(|block_number| *block_number < target_height)
                        .max_by_key(|block_number| *block_number)
                        .map(|block_number| {
                            handle.record_cost(GAS_STORAGE_LOOKUP)?;

                            // At this point, we know the checkpoint exists. So we can safely unwrap.
                            let digest =
                                Checkpoints::<Runtime>::get(chain_key, block_number).unwrap();

                            <EvmResult<HeightHashResult>>::Ok(HeightHashResult {
                                height: block_number,
                                hash: digest,
                                exists: true,
                            })
                        })
                        .transpose()?;

                handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                if maybe_highest.is_some() {
                    break;
                }

                // Move to the next bucket
                block_pivot = block_pivot.saturating_sub(CHECKPOINT_BUCKET_SIZE);
            }

            Ok(maybe_highest.unwrap_or_default())
        } else {
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

            let mut items_processed = 0_u64;

            // If the target height is lower than the last checkpoint height, we first search through the attestations directly.
            let highest = if let Some(highest) = Attestations::<Runtime>::iter_prefix(chain_key)
                .inspect(|_| {
                    items_processed += 1;
                })
                .filter(|(_, attestation)| attestation.header_number() < target_height)
                .max_by_key(|(height, _)| *height)
                .map(|(hash, attestation)| HeightHashResult {
                    height: attestation.header_number(),
                    hash,
                    exists: true,
                }) {
                highest
            } else if let Some(last_checkpoint_height) = maybe_last_checkpoint_height {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                // If we didn't find any attestations below the target height, we fall back to the last checkpoint.
                let digest = Checkpoints::<Runtime>::get(chain_key, last_checkpoint_height)
                    .unwrap_or_default();

                HeightHashResult {
                    height: last_checkpoint_height,
                    hash: digest,
                    exists: true,
                }
            } else {
                // If there are no attestations and no checkpoints, return default.
                HeightHashResult::default()
            };

            handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

            Ok(highest)
        }
    }

    #[precompile::public("find_lowest_attested_after(uint64,uint64)")]
    #[precompile::view]
    fn find_lowest_attested_after(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        target_height: u64,
    ) -> EvmResult<HeightHashResult> {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let maybe_last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        // We first check if the latest checkpoint height is higher to the target height.
        if matches!(maybe_last_checkpoint_height, Some(height) if height > target_height) {
            // If it is search through the checkpoints to find the lowest attested height above the target.

            // We search through the bucket of the block corresponding to target_height for any checkpoints.
            let mut block_pivot =
                PalletAttestationPoc::<Runtime>::compute_block_index_for(target_height);

            let mut maybe_lowest = None;

            // We limit the number of bucket searches to avoid excessive computation.
            for _ in 0..BUCKET_SEARCH_ATTEMPTS {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                let mut items_processed = 0_u64;

                // We search the checkpoint bucket for the lowest checkpoint above the target height.
                maybe_lowest =
                    CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, block_pivot))
                        .inspect(|_| {
                            items_processed += 1;
                        })
                        .filter(|block_number| *block_number >= target_height)
                        .min_by_key(|block_number| *block_number)
                        .map(|block_number| {
                            handle.record_cost(GAS_STORAGE_LOOKUP)?;

                            // At this point, we know the checkpoint exists. So we can safely unwrap.
                            let digest =
                                Checkpoints::<Runtime>::get(chain_key, block_number).unwrap();
                            <EvmResult<HeightHashResult>>::Ok(HeightHashResult {
                                height: block_number,
                                hash: digest,
                                exists: true,
                            })
                        })
                        .transpose()?;

                handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                if maybe_lowest.is_some() {
                    break;
                }

                // Move to the next bucket
                block_pivot = block_pivot.saturating_add(CHECKPOINT_BUCKET_SIZE);
            }

            Ok(maybe_lowest.unwrap_or_default())
        } else {
            // This is the lookup of the first iter_prefix below.
            handle.record_cost(GAS_STORAGE_LOOKUP)?;

            let mut items_processed = 0_u64;

            // Otherwise if the latest checkpoint is below or at the target height, we search through the attestations directly.
            let lowest = Attestations::<Runtime>::iter_prefix(chain_key)
                .inspect(|_| {
                    items_processed += 1;
                })
                .filter(|(_, attestation)| attestation.header_number() >= target_height)
                .min_by_key(|(height, _)| *height)
                .map(|(hash, attestation)| HeightHashResult {
                    height: attestation.header_number(),
                    hash,
                    exists: true,
                })
                .unwrap_or_default();

            handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

            Ok(lowest)
        }
    }

    #[precompile::public("is_height_attested(uint64,uint64)")]
    #[precompile::view]
    fn is_height_attested(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        target_height: u64,
    ) -> EvmResult<bool> {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let maybe_last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        let (found_prev, found_next) = match maybe_last_checkpoint_height {
            Some(last_checkpoint_height) if last_checkpoint_height < target_height => {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                let mut items_processed = 0_u64;

                // We check through the attestations for any attestation above (or at) the target height.
                let found_next_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() >= target_height);

                handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                // Since the last checkpoint is below the target height, if we found any attestation above (or at) the target height,
                // we can be sure that target height is attested.
                (true, found_next_attestation)
            }
            Some(last_checkpoint_height) if last_checkpoint_height > target_height => {
                // We search through the bucket of the block corresponding to target_height - 1 for any checkpoints.
                let mut block_pivot = PalletAttestationPoc::<Runtime>::compute_block_index_for(
                    target_height.saturating_sub(1),
                );

                let mut found_prev_checkpoint = false;

                // We limit the number of bucket searches to avoid excessive computation.
                for _ in 0..BUCKET_SEARCH_ATTEMPTS {
                    handle.record_cost(GAS_STORAGE_LOOKUP)?;

                    let mut items_processed = 0_u64;

                    found_prev_checkpoint =
                        CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, block_pivot))
                            .inspect(|_| {
                                items_processed += 1;
                            })
                            .any(|block_number| block_number <= target_height);

                    handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                    if found_prev_checkpoint {
                        break;
                    }

                    // Move to the next bucket
                    block_pivot = block_pivot.saturating_sub(CHECKPOINT_BUCKET_SIZE);
                }

                // Since the last checkpoint is above the target height, if we found any checkpoint below (or at) the target height,
                // we can be sure that target height is attested.
                (found_prev_checkpoint, true)
            }
            Some(_) => {
                // If the last checkpoint is exactly at the target height, then we know for sure that the target height is attested.
                (true, true)
            }
            None => {
                // We have no checkpoints, so we check through the attestations directly.
                handle.record_cost(GAS_STORAGE_LOOKUP)?;
                let mut items_processed = 0_u64;

                let found_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() == target_height);

                handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                if found_attestation {
                    // If we found an attestation exactly at the target height, we can be sure that target height is attested.
                    return Ok(true);
                }

                handle.record_cost(GAS_STORAGE_LOOKUP)?;
                items_processed = 0_u64;

                // We check through the attestations for any attestation above (or at) the target height.
                let found_next_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() > target_height);

                handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                handle.record_cost(GAS_STORAGE_LOOKUP)?;
                items_processed = 0_u64;

                // We check through the attestations for any attestation above (or at) the target height.
                let found_prev_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() < target_height);

                handle.record_cost(GAS_PER_ITERATION_ITEM * items_processed)?;

                (found_next_attestation, found_prev_attestation)
            }
        };

        Ok(found_prev && found_next)
    }

    #[precompile::public("get_attestation_bounds(uint64,uint64)")]
    fn get_attestation_bounds(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        target_height: u64,
    ) -> EvmResult<BoundsCheckResult> {
        let prev_attestation =
            Self::find_highest_attested_before(handle, chain_key, target_height)?;
        let next_attestation = Self::find_lowest_attested_after(handle, chain_key, target_height)?;

        let bounds = BoundsCheckResult {
            parent: prev_attestation.height,
            parent_hash: prev_attestation.hash,
            parent_is_attestation: prev_attestation.exists,
            child: next_attestation.height,
            child_hash: next_attestation.hash,
            child_is_attestation: next_attestation.exists,
            is_attested: prev_attestation.exists && next_attestation.exists,
        };

        Ok(bounds)
    }
}
