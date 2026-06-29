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
use pallet_attestation::{
    AttestationChainGenesisBlockNumber, Attestations, CheckpointBuckets, LastCheckpoint,
    LastDigest, Pallet as PalletAttestationPoc, CHECKPOINT_BUCKET_SIZE,
};
use pallet_evm::AddressMapping;
use pallet_supported_chains::SupportedChains;
use precompile_utils::{prelude::*, solidity::Codec};

// Gas cost constants
/// Cost of each storage read (matches cold SLOAD) in gas. Also charged per item pulled from a
/// storage-prefix iteration: each item is a full key+value trie read, so it must be priced as a
/// cold lookup, otherwise a large prefix scan is charged far under its real DB cost.
pub const GAS_STORAGE_LOOKUP: u64 = 2_600;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Precompile exposing source chain info to the evm.
pub struct ChainInfoPrecompile<Runtime>(PhantomData<Runtime>);

/// Defensive hard cap on how many items a single `Attestations::iter_prefix` scan will pull.
///
/// The scans below are already priced correctly (each item charges `GAS_STORAGE_LOOKUP`), so an
/// oversized store self-limits by exhausting gas. This cap is a belt-and-suspenders guard so the
/// iterator can never run unbounded even if gas accounting is ever changed or bypassed; it is set
/// far above any realistic per-chain attestation count, so normal-sized stores are unaffected.
const MAX_ATTESTATION_SCAN_ITEMS: u64 = 100_000;

/// Compute how many checkpoint buckets to walk to cover a search span of `span_blocks` blocks.
///
/// Bounding the walk by the *real* search span (rather than a fixed constant) is what fixes the
/// sparse-checkpoint false negative: a valid checkpoint far from the target is always reachable,
/// no matter how far it sits from the target. The result is `ceil(span_blocks /
/// CHECKPOINT_BUCKET_SIZE) + 1` (the `+1` covers the partial bucket at each end).
///
/// The caller is responsible for passing the span that matches the walk *direction*:
/// - a downward walk (toward genesis) spans from `target_height` down to block 0, i.e.
///   `span_blocks = target_height`;
/// - an upward walk (toward the chain tip) spans from `target_height` up to the last checkpoint,
///   i.e. `span_blocks = last_checkpoint_height - target_height`.
///
/// Using `abs_diff(target_height, last_checkpoint_height)` for a *downward* walk is wrong: when the
/// last checkpoint sits just above the target the distance is tiny, yet the nearest checkpoint
/// below the target can be many buckets away, so the walk would stop early and false-negative.
///
/// There is deliberately **no** fixed upper clamp: an arbitrary ceiling (the previous `4096`)
/// re-introduces the exact false-negative this fixes for high block heights (a target above
/// `4096 * CHECKPOINT_BUCKET_SIZE` would stop short of genesis). The loop is instead bounded by
/// gas — every bucket iteration charges `GAS_STORAGE_LOOKUP`, so a pathologically deep walk
/// exhausts gas and *reverts* (an honest failure) rather than silently returning "not attested".
/// The `u32::MAX` saturation only guards the loop counter's type; gas is the real bound.
fn bucket_search_attempts(span_blocks: u64) -> u32 {
    // ceil(span_blocks / CHECKPOINT_BUCKET_SIZE) + 1
    let buckets = span_blocks
        .div_ceil(CHECKPOINT_BUCKET_SIZE)
        .saturating_add(1);
    buckets.try_into().unwrap_or(u32::MAX)
}

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
pub struct HashResult {
    pub hash: H256,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct HeightHashResult {
    pub height: u64,
    pub hash: H256,
    pub is_attestation: bool,
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

/// Probe `SupportedChains` for `chain_key`. Charges a single storage lookup and reverts the
/// call if the chain isn't registered.
///
/// Why revert rather than return a zero default: most methods here have `exists: false` sentinels
/// already used for "this height has no data yet" — a Solidity caller can't distinguish that
/// from "the chain_key you passed is bogus" if both fall through to the default. Reverting
/// surfaces the input error immediately (and refunds remaining gas, which is cheaper than
/// burning the rest of the method's lookups against an unsupported key).
fn ensure_chain_supported<Runtime: pallet_supported_chains::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
) -> EvmResult<()> {
    handle.record_cost(GAS_STORAGE_LOOKUP)?;
    if !SupportedChains::<Runtime>::contains_key(chain_key) {
        return Err(RevertReason::custom("chain not supported").into());
    }
    Ok(())
}

#[precompile_utils::precompile]
impl<Runtime> ChainInfoPrecompile<Runtime>
where
    Runtime: pallet_supported_chains::Config + pallet_evm::Config + pallet_attestation::Config,
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
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

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
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        if let Some(last_digest) = LastDigest::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(last_digest.encoded_size())?;

            Ok(HeightHashResult {
                height: last_digest.0,
                hash: last_digest.1,
                is_attestation: true,
                exists: true,
            })
        } else if let Some(last_checkpoint) = LastCheckpoint::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(last_checkpoint.encoded_size())?;

            Ok(HeightHashResult {
                height: last_checkpoint.block_number,
                hash: last_checkpoint.digest,
                is_attestation: false,
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
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        if let Some(last_checkpoint) = LastCheckpoint::<Runtime>::get(chain_key) {
            handle.record_db_read::<Runtime>(last_checkpoint.encoded_size())?;

            Ok(HeightHashResult {
                height: last_checkpoint.block_number,
                hash: last_checkpoint.digest,
                is_attestation: false,
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
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let maybe_last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        // We first check if the latest checkpoint height is higher or equal to the target height.
        if maybe_last_checkpoint_height.is_some_and(|height| height >= target_height) {
            // If it is search through the checkpoints to find the highest attested height below the target.
            // We search through the bucket of the block corresponding to target_height - 1 for any checkpoints.
            let mut block_pivot = PalletAttestationPoc::<Runtime>::compute_block_index_for(
                target_height.saturating_sub(1),
            );

            let mut maybe_highest = None;

            // This walks *downward* from `target_height - 1` toward block 0, so the span is
            // `target_height` (a checkpoint below the target can sit anywhere down to genesis,
            // independent of where the last checkpoint is). The loop is bounded by gas
            // (each bucket charges `GAS_STORAGE_LOOKUP`), not an artificial attempt ceiling.
            let attempts = bucket_search_attempts(target_height);
            for _ in 0..attempts {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                let mut items_processed = 0_u64;

                // Collect this bucket's heights below the target, then probe them from
                // highest downward. `CheckpointBuckets` is expected to mirror `Checkpoints`
                // (writes are paired), but the prefix clears in `clear_or_revert.rs` advance
                // the two maps' cursors independently during chain removal, so a bucket can
                // transiently hold heights whose checkpoint was already cleared. Skip such
                // orphans and keep probing *within* the bucket — only fall through to the
                // next pivot once every height here is exhausted. Probing extremal-first with
                // an early break keeps the common (no-desync) case at a single lookup.
                let mut candidates: Vec<u64> =
                    CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, block_pivot))
                        .inspect(|_| {
                            items_processed += 1;
                        })
                        .filter(|block_number| *block_number < target_height)
                        .collect();
                handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

                // Highest first.
                candidates.sort_unstable_by(|a, b| b.cmp(a));

                maybe_highest = None;
                for block_number in candidates {
                    // `checkpoint_if_stable` performs up to two storage reads —
                    // `CheckpointPruningStates::get` followed by `Checkpoints::get` — so charge
                    // both lookups unconditionally (matches `block-prover::get_checkpoint`).
                    handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;
                    if let Some(digest) = PalletAttestationPoc::<Runtime>::checkpoint_if_stable(
                        chain_key,
                        block_number,
                    ) {
                        maybe_highest = Some(HeightHashResult {
                            height: block_number,
                            hash: digest,
                            is_attestation: false,
                            exists: true,
                        });
                        break;
                    }
                }

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
                // Defensive hard cap (see MAX_ATTESTATION_SCAN_ITEMS): bound the scan even if gas
                // accounting is bypassed; far above any realistic store, so normal scans are unaffected.
                .take(MAX_ATTESTATION_SCAN_ITEMS as usize)
                .inspect(|_| {
                    items_processed += 1;
                })
                .filter(|(_, attestation)| attestation.header_number() < target_height)
                .max_by_key(|(_, attestation)| attestation.header_number())
                .map(|(hash, attestation)| HeightHashResult {
                    height: attestation.header_number(),
                    hash,
                    is_attestation: true,
                    exists: true,
                }) {
                highest
            } else if let Some(last_checkpoint_height) = maybe_last_checkpoint_height {
                // `checkpoint_if_stable` does the `CheckpointPruningStates` guard read plus the
                // `Checkpoints` read — charge both.
                handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;

                // If we didn't find any attestations below the target height, we fall back to
                // the last checkpoint. `checkpoint_if_stable` returns `None` when the height's
                // pivot is still being drained post-revert; surface that as `exists: false`
                // rather than `unwrap_or_default()`-ing to a zero digest with `exists: true`,
                // which would advertise a stale-but-withheld digest as live data. Matches
                // `get_checkpoint_for_height`'s behaviour for the same gated read.
                if let Some(digest) = PalletAttestationPoc::<Runtime>::checkpoint_if_stable(
                    chain_key,
                    last_checkpoint_height,
                ) {
                    HeightHashResult {
                        height: last_checkpoint_height,
                        hash: digest,
                        is_attestation: false,
                        exists: true,
                    }
                } else {
                    HeightHashResult::default()
                }
            } else {
                // If there are no attestations and no checkpoints, return default.
                HeightHashResult::default()
            };

            handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

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
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let maybe_last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        // We first check if the latest checkpoint height is higher to the target height.
        if let Some(last_checkpoint_height) =
            maybe_last_checkpoint_height.filter(|height| *height > target_height)
        {
            // If it is search through the checkpoints to find the lowest attested height above the target.

            // We search through the bucket of the block corresponding to target_height for any checkpoints.
            let mut block_pivot =
                PalletAttestationPoc::<Runtime>::compute_block_index_for(target_height);

            let mut maybe_lowest = None;

            // This walks *upward* from `target_height` toward the chain tip; the highest reachable
            // checkpoint is the last checkpoint, so the span is `last_checkpoint_height -
            // target_height`. This branch only runs when `last_checkpoint_height > target_height`.
            // Bounded by gas per bucket, not an artificial attempt ceiling.
            let attempts =
                bucket_search_attempts(last_checkpoint_height.saturating_sub(target_height));
            for _ in 0..attempts {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                let mut items_processed = 0_u64;

                // See the matching note in `find_highest_attested_before`: probe within the
                // bucket (lowest first) and skip orphans whose `Checkpoints` entry was already
                // cleared, only falling through to the next pivot once this bucket is exhausted.
                let mut candidates: Vec<u64> =
                    CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, block_pivot))
                        .inspect(|_| {
                            items_processed += 1;
                        })
                        .filter(|block_number| *block_number >= target_height)
                        .collect();
                handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

                // Lowest first.
                candidates.sort_unstable();

                maybe_lowest = None;
                for block_number in candidates {
                    // See the matching note in `find_highest_attested_before`: the helper does
                    // up to two storage reads, so charge both lookups unconditionally.
                    handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;
                    if let Some(digest) = PalletAttestationPoc::<Runtime>::checkpoint_if_stable(
                        chain_key,
                        block_number,
                    ) {
                        maybe_lowest = Some(HeightHashResult {
                            height: block_number,
                            hash: digest,
                            is_attestation: false,
                            exists: true,
                        });
                        break;
                    }
                }

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
                // Defensive hard cap (see MAX_ATTESTATION_SCAN_ITEMS): bound the scan even if gas
                // accounting is bypassed; far above any realistic store, so normal scans are unaffected.
                .take(MAX_ATTESTATION_SCAN_ITEMS as usize)
                .inspect(|_| {
                    items_processed += 1;
                })
                .filter(|(_, attestation)| attestation.header_number() >= target_height)
                .min_by_key(|(_, attestation)| attestation.header_number())
                .map(|(hash, attestation)| HeightHashResult {
                    height: attestation.header_number(),
                    hash,
                    is_attestation: true,
                    exists: true,
                })
                .unwrap_or_default();

            handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

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
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let maybe_last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        let (found_prev, found_next) = match maybe_last_checkpoint_height {
            Some(last_checkpoint_height) if last_checkpoint_height < target_height => {
                handle.record_cost(GAS_STORAGE_LOOKUP)?;

                let mut items_processed = 0_u64;

                // We check through the attestations for any attestation above (or at) the target height.
                let found_next_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    // Defensive hard cap (see MAX_ATTESTATION_SCAN_ITEMS): bound the scan even if gas
                    // accounting is bypassed; far above any realistic store, so normal scans are unaffected.
                    .take(MAX_ATTESTATION_SCAN_ITEMS as usize)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() >= target_height);

                handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

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

                // This walks *downward* from `target_height - 1` toward block 0 (same direction as
                // `find_highest_attested_before`), so the span is `target_height`, not the
                // distance to the last checkpoint. Bounded by gas per bucket, not a fixed ceiling.
                let attempts = bucket_search_attempts(target_height);
                for _ in 0..attempts {
                    handle.record_cost(GAS_STORAGE_LOOKUP)?;

                    let mut items_processed = 0_u64;

                    found_prev_checkpoint =
                        CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, block_pivot))
                            .inspect(|_| {
                                items_processed += 1;
                            })
                            .any(|block_number| block_number <= target_height);

                    handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

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
                    // Defensive hard cap (see MAX_ATTESTATION_SCAN_ITEMS): bound the scan even if gas
                    // accounting is bypassed; far above any realistic store, so normal scans are unaffected.
                    .take(MAX_ATTESTATION_SCAN_ITEMS as usize)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() == target_height);

                handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

                if found_attestation {
                    // If we found an attestation exactly at the target height, we can be sure that target height is attested.
                    return Ok(true);
                }

                handle.record_cost(GAS_STORAGE_LOOKUP)?;
                items_processed = 0_u64;

                // We check through the attestations for any attestation above (or at) the target height.
                let found_next_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    // Defensive hard cap (see MAX_ATTESTATION_SCAN_ITEMS): bound the scan even if gas
                    // accounting is bypassed; far above any realistic store, so normal scans are unaffected.
                    .take(MAX_ATTESTATION_SCAN_ITEMS as usize)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() > target_height);

                handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

                handle.record_cost(GAS_STORAGE_LOOKUP)?;
                items_processed = 0_u64;

                // We check through the attestations for any attestation above (or at) the target height.
                let found_prev_attestation = Attestations::<Runtime>::iter_prefix(chain_key)
                    // Defensive hard cap (see MAX_ATTESTATION_SCAN_ITEMS): bound the scan even if gas
                    // accounting is bypassed; far above any realistic store, so normal scans are unaffected.
                    .take(MAX_ATTESTATION_SCAN_ITEMS as usize)
                    .inspect(|_| {
                        items_processed += 1;
                    })
                    .any(|(_, attestation)| attestation.header_number() < target_height);

                handle.record_cost(GAS_STORAGE_LOOKUP * items_processed)?;

                (found_next_attestation, found_prev_attestation)
            }
        };

        Ok(found_prev && found_next)
    }

    #[precompile::public("get_attestation_bounds(uint64,uint64)")]
    #[precompile::view]
    fn get_attestation_bounds(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        target_height: u64,
    ) -> EvmResult<BoundsCheckResult> {
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        // `find_highest_attested_before` / `find_lowest_attested_after` each repeat the
        // supported-chain check internally. For the supported path that's 2 extra lookups; for
        // the unsupported path the revert above means they never run at all. Worth the small
        // redundancy to keep the inner methods self-guarded when called externally.
        let prev_attestation =
            Self::find_highest_attested_before(handle, chain_key, target_height)?;
        let next_attestation = Self::find_lowest_attested_after(handle, chain_key, target_height)?;

        let bounds = BoundsCheckResult {
            parent: prev_attestation.height,
            parent_hash: prev_attestation.hash,
            parent_is_attestation: prev_attestation.is_attestation,
            child: next_attestation.height,
            child_hash: next_attestation.hash,
            child_is_attestation: next_attestation.is_attestation,
            is_attested: prev_attestation.exists && next_attestation.exists,
        };

        Ok(bounds)
    }

    #[precompile::public("get_attestation_height_for_digest(uint64,bytes32)")]
    #[precompile::view]
    fn get_attestation_height_for_digest(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        digest: H256,
    ) -> EvmResult<HeightResult> {
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        if let Some(attestation) = Attestations::<Runtime>::get(chain_key, digest) {
            Ok(HeightResult {
                height: attestation.header_number(),
                exists: true,
            })
        } else {
            Ok(HeightResult::default())
        }
    }

    #[precompile::public("get_checkpoint_for_height(uint64,uint64)")]
    #[precompile::view]
    fn get_checkpoint_for_height(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
        height: u64,
    ) -> EvmResult<HashResult> {
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        // `checkpoint_if_stable` does the `CheckpointPruningStates` guard read plus the
        // `Checkpoints` read — charge both (matches `block-prover::get_checkpoint`).
        handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;

        if let Some(digest) =
            PalletAttestationPoc::<Runtime>::checkpoint_if_stable(chain_key, height)
        {
            Ok(HashResult {
                hash: digest,
                exists: true,
            })
        } else {
            Ok(HashResult::default())
        }
    }
}
