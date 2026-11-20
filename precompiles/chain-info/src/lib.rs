#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
    sp_runtime::traits::Dispatchable,
};
use sp_core::{Encode, H160, H256, U256};
use sp_std::vec::Vec;

use attestor_primitives::{AttestationCheckpoint, ChainId, ChainKey};
use pallet_attestation::{
    AttestationChainGenesisBlockNumber, Attestations, CheckpointBuckets, LastCheckpoint,
    LastDigest, Pallet as PalletAttestationPoc, CHECKPOINT_BUCKET_SIZE,
};
use pallet_evm::AddressMapping;
use pallet_supported_chains::{CoreFees, OutboxDiscoveries, OutboxFactories, SupportedChains};
use parity_scale_codec::MaxEncodedLen;
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
pub struct OutboxFactoryResult {
    pub factory_addr: Address,
    pub exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Codec)]
pub struct OutboxDiscoveryResult {
    pub discovery_addr: Address,
    pub exists: bool,
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

/// Visit `(digest, header_number)` for the retained attestations of `chain_key`; `visit` returns
/// `true` to stop. Each row is charged one lookup *before* its value is read and decoded, so a
/// call that cannot pay for the traversal stops at the first row it cannot afford.
///
/// Visits nothing while a cleanup cursor is draining `chain_key`'s `Attestations`: the cursor
/// sweeps the prefix in hash order, so any row found could be stale post-revert data. The
/// `attestations_stable` guard and the first step of the iteration are charged up front.
fn scan_attestations<Runtime: pallet_attestation::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
    mut visit: impl FnMut(H256, u64) -> bool,
) -> EvmResult<()> {
    handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;
    if !PalletAttestationPoc::<Runtime>::attestations_stable(chain_key) {
        return Ok(());
    }

    for digest in Attestations::<Runtime>::iter_key_prefix(chain_key) {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;
        if let Some(attestation) = Attestations::<Runtime>::get(chain_key, digest) {
            if visit(digest, attestation.header_number()) {
                break;
            }
        }
    }
    Ok(())
}

/// Whether any retained attestation of `chain_key` has a header number accepted by `matches`.
fn any_attestation<Runtime: pallet_attestation::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
    matches: impl Fn(u64) -> bool,
) -> EvmResult<bool> {
    let mut found = false;
    scan_attestations::<Runtime>(handle, chain_key, |_, height| {
        found = matches(height);
        found
    })?;
    Ok(found)
}

/// Heights recorded in the checkpoint bucket at `pivot` that `keep` accepts. Each recorded height
/// is charged one lookup before it is examined.
fn bucket_heights<Runtime: pallet_attestation::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
    pivot: u64,
    keep: impl Fn(u64) -> bool,
) -> EvmResult<Vec<u64>> {
    let mut heights = Vec::new();
    for block_number in CheckpointBuckets::<Runtime>::iter_key_prefix((chain_key, pivot)) {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;
        if keep(block_number) {
            heights.push(block_number);
        }
    }
    Ok(heights)
}

/// The stable checkpoint at `block_number`, as a result entry. Charges the two reads
/// `checkpoint_if_stable` performs (the `CheckpointPruningStates` guard and `Checkpoints`).
fn stable_checkpoint<Runtime: pallet_attestation::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
    block_number: u64,
) -> EvmResult<Option<HeightHashResult>> {
    handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;
    Ok(
        PalletAttestationPoc::<Runtime>::checkpoint_if_stable(chain_key, block_number).map(
            |digest| HeightHashResult {
                height: block_number,
                hash: digest,
                is_attestation: false,
                exists: true,
            },
        ),
    )
}

/// Highest stable checkpoint at or below `max_height`, walking the checkpoint buckets downward
/// from `max_height`'s.
///
/// `CheckpointBuckets` mirrors `Checkpoints`, but chain-removal clears advance the two cursors
/// independently, so a bucket can briefly hold heights whose checkpoint is gone. Orphans are
/// skipped and probing continues within the bucket, highest first.
fn highest_checkpoint_at_or_below<Runtime: pallet_attestation::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
    max_height: u64,
) -> EvmResult<Option<HeightHashResult>> {
    let mut block_pivot = PalletAttestationPoc::<Runtime>::compute_block_index_for(max_height);

    // We limit the number of bucket searches to avoid excessive computation.
    for _ in 0..BUCKET_SEARCH_ATTEMPTS {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let mut candidates =
            bucket_heights::<Runtime>(handle, chain_key, block_pivot, |h| h <= max_height)?;
        candidates.sort_unstable_by(|a, b| b.cmp(a));

        for block_number in candidates {
            if let Some(found) = stable_checkpoint::<Runtime>(handle, chain_key, block_number)? {
                return Ok(Some(found));
            }
        }

        // Move to the next bucket
        block_pivot = block_pivot.saturating_sub(CHECKPOINT_BUCKET_SIZE);
    }

    Ok(None)
}

/// Lowest stable checkpoint at or above `target_height`, walking the checkpoint buckets upward
/// from the target's. Mirror of [`highest_checkpoint_at_or_below`].
fn lowest_checkpoint_from<Runtime: pallet_attestation::Config>(
    handle: &mut impl PrecompileHandle,
    chain_key: ChainKey,
    target_height: u64,
) -> EvmResult<Option<HeightHashResult>> {
    let mut block_pivot = PalletAttestationPoc::<Runtime>::compute_block_index_for(target_height);

    // We limit the number of bucket searches to avoid excessive computation.
    for _ in 0..BUCKET_SEARCH_ATTEMPTS {
        handle.record_cost(GAS_STORAGE_LOOKUP)?;

        let mut candidates =
            bucket_heights::<Runtime>(handle, chain_key, block_pivot, |h| h >= target_height)?;
        candidates.sort_unstable();

        for block_number in candidates {
            if let Some(found) = stable_checkpoint::<Runtime>(handle, chain_key, block_number)? {
                return Ok(Some(found));
            }
        }

        // Move to the next bucket
        block_pivot = block_pivot.saturating_add(CHECKPOINT_BUCKET_SIZE);
    }

    Ok(None)
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
        // The lookup is a real DB read whether or not `chain_key` resolves, so both arms must
        // charge it (matches `AttestorStashPrecompile::get_attestor`).
        match SupportedChains::<Runtime>::get(chain_key) {
            Some(sc) => {
                handle.record_db_read::<Runtime>(sc.encoded_size())?;
                let chain = ChainInfo {
                    chain_key,
                    chain_id: sc.chain_id,
                    chain_name: UnboundedBytes::from(sc.chain_name),
                    chain_encoding: sc.chain_encoding as u8,
                };

                Ok(ChainInfoResult::with_chain(chain))
            }
            None => {
                handle.record_db_read::<Runtime>(0)?;
                // We want an empty return rather than a revert here
                Ok(ChainInfoResult::default())
            }
        }
    }

    #[precompile::public("get_outbox_factory_address(uint64)")]
    #[precompile::view]
    fn get_outbox_factory_address(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<OutboxFactoryResult> {
        let maybe_address = OutboxFactories::<Runtime>::get(chain_key);

        handle.record_db_read::<Runtime>(
            maybe_address
                .as_ref()
                .map(|address| address.encoded_size())
                .unwrap_or_default(),
        )?;

        match maybe_address {
            Some(address) => Ok(OutboxFactoryResult {
                factory_addr: Address(address),
                exists: true,
            }),
            None => Ok(OutboxFactoryResult {
                factory_addr: Address(H160::zero()),
                exists: false,
            }),
        }
    }

    /// Outbox discovery-registry address for `chain_key` — the `OutboxDeployer`/`OutboxDiscovery`
    /// in asc-contracts, whose `outboxOf`/`defaultOutbox` getter is the authoritative source for
    /// the current Outbox. Unlike [`get_outbox_factory_address`](Self::get_outbox_factory_address),
    /// which only identifies the permissionless factory contract, this address is safe for a
    /// resolver to trust: registry entries are only written through an access-controlled deploy
    /// path, so a caller can bind the Outbox from this registry instead of scanning the factory's
    /// `OutboxCreated` logs, which an attacker can spoof for any chain key.
    #[precompile::public("get_outbox_discovery_address(uint64)")]
    #[precompile::view]
    fn get_outbox_discovery_address(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<OutboxDiscoveryResult> {
        let maybe_address = OutboxDiscoveries::<Runtime>::get(chain_key);

        handle.record_db_read::<Runtime>(
            maybe_address
                .as_ref()
                .map(|address| address.encoded_size())
                .unwrap_or_default(),
        )?;

        match maybe_address {
            Some(address) => Ok(OutboxDiscoveryResult {
                discovery_addr: Address(address),
                exists: true,
            }),
            None => Ok(OutboxDiscoveryResult {
                discovery_addr: Address(H160::zero()),
                exists: false,
            }),
        }
    }

    /// USC write-ability core (protocol) fee for `chain_key`, in **attestcoin wei**, read live from
    /// pallet storage so the Outbox charges the current governance-set value on every
    /// `publishMessage` without a redeploy. An unconfigured chain returns `0`, which the consumer
    /// treats as "no fee" — the same outcome as an explicit zero amount, so no separate `exists`
    /// flag is needed.
    ///
    /// # ABI contract
    ///
    /// This is consumed by `ICoreFeeProvider` in `usc-contracts` (behind `IFeeRegistry`, which the
    /// Outbox depends on), which declares exactly:
    ///
    /// ```solidity
    /// function get_core_fee(uint32 chainKey) external view returns (uint256);
    /// ```
    ///
    /// so the signature — including the `uint32` argument width, which is narrower than the runtime's
    /// [`ChainKey`] and is what the contracts use for chain keys — and the bare `uint256` return must
    /// not drift: selector `0x5b023376`. There is deliberately no token field; see
    /// [`CoreFeeConfig`](supported_chains_primitives::CoreFeeConfig) for why the denomination is
    /// fixed to attestcoin.
    #[precompile::public("get_core_fee(uint32)")]
    #[precompile::view]
    fn get_core_fee(handle: &mut impl PrecompileHandle, chain_key: u32) -> EvmResult<U256> {
        // Widening u32 -> ChainKey (u64) is lossless; the narrower argument exists only to match the
        // Solidity consumer's chainKey type.
        let maybe_fee = CoreFees::<Runtime>::get(ChainKey::from(chain_key));

        handle.record_db_read::<Runtime>(
            maybe_fee
                .as_ref()
                .map(|fee| fee.encoded_size())
                .unwrap_or_default(),
        )?;

        Ok(maybe_fee.map(|fee| fee.amount).unwrap_or_else(U256::zero))
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

        // Each read is charged whether or not the entry exists, at its fixed encoded size.
        handle.record_db_read::<Runtime>(<(u64, H256)>::max_encoded_len())?;
        if let Some(last_digest) = LastDigest::<Runtime>::get(chain_key) {
            return Ok(HeightHashResult {
                height: last_digest.0,
                hash: last_digest.1,
                is_attestation: true,
                exists: true,
            });
        }

        handle.record_db_read::<Runtime>(AttestationCheckpoint::max_encoded_len())?;
        if let Some(last_checkpoint) = LastCheckpoint::<Runtime>::get(chain_key) {
            return Ok(HeightHashResult {
                height: last_checkpoint.block_number,
                hash: last_checkpoint.digest,
                is_attestation: false,
                exists: true,
            });
        }

        Ok(HeightHashResult::default())
    }

    #[precompile::public("get_latest_checkpoint_height_and_hash(uint64)")]
    #[precompile::view]
    fn get_latest_checkpoint_height_and_hash(
        handle: &mut impl PrecompileHandle,
        chain_key: ChainKey,
    ) -> EvmResult<HeightHashResult> {
        ensure_chain_supported::<Runtime>(handle, chain_key)?;

        handle.record_db_read::<Runtime>(AttestationCheckpoint::max_encoded_len())?;
        if let Some(last_checkpoint) = LastCheckpoint::<Runtime>::get(chain_key) {
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

        let last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        // Condensed attestations stay in storage for the retention period, so one can be closer
        // to the target than the nearest checkpoint: look at both and keep the nearest. A
        // checkpoint wins a tie, as it outlives the attestation.
        let mut attestation: Option<HeightHashResult> = None;
        scan_attestations::<Runtime>(handle, chain_key, |hash, height| {
            if height < target_height
                && attestation.as_ref().is_none_or(|best| height > best.height)
            {
                attestation = Some(HeightHashResult {
                    height,
                    hash,
                    is_attestation: true,
                    exists: true,
                });
            }
            false
        })?;

        let checkpoint = match last_checkpoint_height {
            Some(last) if last >= target_height => match target_height.checked_sub(1) {
                Some(max_height) => {
                    highest_checkpoint_at_or_below::<Runtime>(handle, chain_key, max_height)?
                }
                // Nothing lies below height 0.
                None => None,
            },
            // Every checkpoint is below the target, so the last one is the nearest. It is only
            // read when it can matter: an attestation above it wins.
            // `None` while the height's pivot is still being drained post-revert: surfaced as
            // `exists: false`, not a zero digest advertised as live data.
            Some(last) if attestation.as_ref().is_none_or(|best| best.height <= last) => {
                stable_checkpoint::<Runtime>(handle, chain_key, last)?
            }
            _ => None,
        };

        Ok(match (attestation, checkpoint) {
            (Some(attestation), Some(checkpoint)) if attestation.height > checkpoint.height => {
                attestation
            }
            (_, Some(checkpoint)) => checkpoint,
            (Some(attestation), None) => attestation,
            (None, None) => HeightHashResult::default(),
        })
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

        let last_checkpoint_height =
            LastCheckpoint::<Runtime>::get(chain_key).map(|cp| cp.block_number);

        // See `find_highest_attested_before`: retained attestations and checkpoints compete.
        let mut attestation: Option<HeightHashResult> = None;
        scan_attestations::<Runtime>(handle, chain_key, |hash, height| {
            if height >= target_height
                && attestation.as_ref().is_none_or(|best| height < best.height)
            {
                attestation = Some(HeightHashResult {
                    height,
                    hash,
                    is_attestation: true,
                    exists: true,
                });
            }
            false
        })?;

        // A checkpoint can only be at or above the target when the latest one is.
        let checkpoint = match last_checkpoint_height {
            Some(last) if last >= target_height => {
                lowest_checkpoint_from::<Runtime>(handle, chain_key, target_height)?
            }
            _ => None,
        };

        Ok(match (attestation, checkpoint) {
            (Some(attestation), Some(checkpoint)) if attestation.height < checkpoint.height => {
                attestation
            }
            (_, Some(checkpoint)) => checkpoint,
            (Some(attestation), None) => attestation,
            (None, None) => HeightHashResult::default(),
        })
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
                // We check through the attestations for any attestation above (or at) the
                // target height. Skip while a cleanup cursor is draining `chain_key`'s
                // `Attestations` — any row found could be stale post-revert data — so we can't
                // confirm this bound.
                let found_next_attestation =
                    any_attestation::<Runtime>(handle, chain_key, |height| {
                        height >= target_height
                    })?;

                // Since the last checkpoint is below the target height, if we found any attestation above (or at) the target height,
                // we can be sure that target height is attested.
                (true, found_next_attestation)
            }
            Some(last_checkpoint_height) if last_checkpoint_height > target_height => {
                // Previous bound, cheapest first: a stable checkpoint below the target, found with
                // the same walk as `get_attestation_bounds` (so orphaned or still-draining bucket
                // entries don't count and both search the same buckets); a stable checkpoint
                // exactly at the target; a retained attestation at or below it.
                let found_prev =
                    match target_height.checked_sub(1) {
                        Some(max_height) => highest_checkpoint_at_or_below::<Runtime>(
                            handle, chain_key, max_height,
                        )?
                        .is_some(),
                        None => false,
                    } || stable_checkpoint::<Runtime>(handle, chain_key, target_height)?.is_some()
                        || any_attestation::<Runtime>(handle, chain_key, |height| {
                            height <= target_height
                        })?;

                // Since the last checkpoint is above the target height, if we found any checkpoint or attestation below (or at) the target height,
                // we can be sure that target height is attested.
                (found_prev, true)
            }
            Some(_) => {
                // If the last checkpoint is exactly at the target height, then we know for sure that the target height is attested.
                (true, true)
            }
            None => {
                // We have no checkpoints, so we check through the attestations directly, in a
                // single pass: an attestation exactly at the target proves it, otherwise we need
                // one above and one below. Nothing is found while a cleanup cursor is draining.
                let (mut exact, mut above, mut below) = (false, false, false);
                scan_attestations::<Runtime>(handle, chain_key, |_, height| {
                    match height.cmp(&target_height) {
                        sp_std::cmp::Ordering::Equal => exact = true,
                        sp_std::cmp::Ordering::Greater => above = true,
                        sp_std::cmp::Ordering::Less => below = true,
                    }
                    exact || (above && below)
                })?;

                (exact || below, exact || above)
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
            // Two-sided availability: a nearest parent *and* a nearest child were found.
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

        // `Pallet::get` does the `AttestationClearingCursors` guard read plus the `Attestations`
        // read — charge both (matches `get_checkpoint_for_height`). It returns `None` while a
        // cleanup cursor is draining for `chain_key`, so a stale post-revert row can't be
        // reported as a live attestation height.
        handle.record_cost(GAS_STORAGE_LOOKUP.saturating_mul(2))?;

        if let Some(attestation) = PalletAttestationPoc::<Runtime>::get(chain_key, digest) {
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
