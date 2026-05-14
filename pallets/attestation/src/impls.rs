use frame_support::{
    pallet_prelude::*,
    traits::{ConstU32, Currency, DefensiveSaturating},
    transactional,
};
use log::debug;
use sp_runtime::{
    traits::{CheckedAdd, CheckedSub, SaturatedConversion, Saturating, Zero},
    ArithmeticError,
};
use sp_staking::StakingInterface;
use sp_std::{
    collections::{btree_map::BTreeMap, btree_set::BTreeSet, vec_deque::VecDeque},
    vec::Vec,
};

use attestor_primitives::{
    calculate_threshold, AttestationCheckpoint, Attestor, AttestorStatus, BlsPublicKey,
    BlsSignature, ChainKey, Digest, SignedAttestation,
};
use bls_signatures::key::aggregate_public_keys;
use bls_signatures::{PublicKey, Serialize, Signature};
use randomness_primitives::{OnRandomnessUpdate, Randomness};
use supported_chains_primitives::provider::SupportedChainsProvider;

use crate::{
    asset::existential_deposit,
    ledger::{AttestorLedger, UnlockChunk},
};

use super::pallet::*;

// One tenth of a CTC in micro units
pub const ONE_TENTH_CTC: u64 = 100_000_000_000_000_000;

/// Upper bound on how many checkpoints may sit strictly above a [`forward_patch_checkpoints`] batch tip
/// when [`crate::pallet::Pallet::do_forward_patch_checkpoints`] `wipe_suffix` is enabled (single dispatch).
pub const MAX_CHECKPOINT_SUFFIX_WIPE_TOTAL: usize = 4096;

/// Batch size for scanning removals from [`Attestations`] during [`forward_patch_checkpoints`].
const FORWARD_PATCH_ATTESTATIONS_CLEAR_BATCH: u32 = 128;
/// Upper bound on iterations; total attestations cleared per dispatch ≤ batch × loops (currently 65_536).
const MAX_FORWARD_PATCH_ATTESTATIONS_CLEAR_LOOPS: u32 = 512;

/// PALLET CALL IMPLS ///
impl<T: Config> Pallet<T> {
    pub(crate) fn remove_active_attestor_from_set(chain_key: ChainKey, attestor_id: &T::AccountId) {
        ActiveAttestors::<T>::mutate(chain_key, |active_attestors| {
            if let Some(pos) = active_attestors.iter().position(|x| x == attestor_id) {
                active_attestors.swap_remove(pos);
            }
        });
    }

    /// Drop any retired BLS key row for this controller so a new registration can use the slot.
    pub(crate) fn clear_retired_bls_entry_for_controller(
        chain_key: ChainKey,
        attestor_id: &T::AccountId,
    ) {
        if let Some(entry) = RetiredAttestorBlsKeys::<T>::take(chain_key, attestor_id) {
            Self::remove_stash_retired_index_pair(&entry.stash, chain_key, attestor_id);
            // Release the BLS pubkey claim only if [`BlsKeyOwner`] still points at this
            // controller. If a different controller has since taken the slot (shouldn't
            // happen given the uniqueness gate in `start_attesting`, but defensive), leave
            // it alone.
            if BlsKeyOwner::<T>::get(chain_key, entry.bls_public_key).as_ref() == Some(attestor_id)
            {
                BlsKeyOwner::<T>::remove(chain_key, entry.bls_public_key);
            }
        }
    }

    fn remove_stash_retired_index_pair(
        stash: &T::AccountId,
        chain_key: ChainKey,
        attestor_id: &T::AccountId,
    ) {
        RetiredAttestorKeysByStash::<T>::mutate(stash, |vec| {
            if let Some(pos) = vec
                .iter()
                .position(|(ck, id)| *ck == chain_key && id == attestor_id)
            {
                vec.swap_remove(pos);
            }
        });
    }

    /// After [`RetiredAttestorBlsKeys`] for `chain_key` is cleared (e.g. chain removal), drop
    /// `(chain_key, _)` pairs from [`RetiredAttestorKeysByStash`] so stashes do not retain stale
    /// index entries that could fill the bounded vec.
    pub(crate) fn purge_retired_keys_by_stash_for_removed_chain(chain_key: ChainKey) {
        for stash in RetiredAttestorKeysByStash::<T>::iter().map(|(stash, _)| stash) {
            let pairs = RetiredAttestorKeysByStash::<T>::get(&stash);
            let len_before = pairs.len();
            let filtered: Vec<(ChainKey, T::AccountId)> = pairs
                .into_iter()
                .filter(|(ck, _)| *ck != chain_key)
                .collect();
            if filtered.len() == len_before {
                continue;
            }
            if filtered.is_empty() {
                RetiredAttestorKeysByStash::<T>::remove(&stash);
            } else {
                let kept: BoundedVec<(ChainKey, T::AccountId), ConstU32<64>> = filtered
                    .try_into()
                    .expect("subset of prior bounded vec; qed");
                RetiredAttestorKeysByStash::<T>::insert(&stash, kept);
            }
        }
    }

    fn purge_retired_bls_keys_for_stash(stash: &T::AccountId, current_era: u32) {
        let pairs = RetiredAttestorKeysByStash::<T>::get(stash);
        if pairs.is_empty() {
            return;
        }

        let mut kept = BoundedVec::<(ChainKey, T::AccountId), ConstU32<64>>::default();
        for (chain_key, attestor_id) in pairs.into_iter() {
            match RetiredAttestorBlsKeys::<T>::get(chain_key, &attestor_id) {
                None => {
                    // Dangling index entry only — row already gone.
                }
                Some(entry) if entry.stash != *stash => {
                    // This stash's index points at a BLS row owned by another stash; drop the
                    // stale index entry but do not delete their [`RetiredAttestorBlsKeys`] row.
                }
                Some(entry) => {
                    if current_era >= entry.purge_at_era {
                        // Release the BLS pubkey claim alongside the retired row so the
                        // key can be re-registered by a different controller after the
                        // unbond delay elapses.
                        if BlsKeyOwner::<T>::get(chain_key, entry.bls_public_key).as_ref()
                            == Some(&attestor_id)
                        {
                            BlsKeyOwner::<T>::remove(chain_key, entry.bls_public_key);
                        }
                        RetiredAttestorBlsKeys::<T>::remove(chain_key, &attestor_id);
                    } else {
                        // `kept` is a strict subset of `pairs` which was already bounded to 64 entries,
                        // so this push is infallible.
                        kept.try_push((chain_key, attestor_id))
                            .expect("subset of prior bounded vec; qed");
                    }
                }
            }
        }

        RetiredAttestorKeysByStash::<T>::insert(stash, kept);
    }

    /// Inserts an attestor and sets the default status to Idle
    /// Emits an event `AttestorRegistered` if successful
    /// An attestor needs to call `attest` to become active
    pub fn try_insert_attestor_and_emit_event(
        chain_key: ChainKey,
        stash: T::AccountId,
        attestor_id: T::AccountId,
    ) -> DispatchResult {
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );

        if ChainElectionPolicy::<T>::get(chain_key) == AttestorElectionPolicy::AuthorizedOnly {
            ensure!(
                AuthorizedAttestors::<T>::contains_key(chain_key, &attestor_id),
                Error::<T>::NotPreAuthorizedToRegister
            );
        }

        ensure!(
            !Self::attestor_is_registered(chain_key, &attestor_id),
            Error::<T>::AlreadyAttestor
        );

        Self::clear_retired_bls_entry_for_controller(chain_key, &attestor_id);

        ensure!(
            Self::attestor_list_has_space(chain_key),
            Error::<T>::AttestorListFull
        );

        // Can't select same account as the attestor
        ensure!(attestor_id != stash, Error::<T>::InvalidAttestorAccount);

        // Insert attestor with status Idle
        Attestors::<T>::insert(
            chain_key,
            attestor_id.clone(),
            Attestor {
                bls_public_key: None,
                status: AttestorStatus::Idle,
                stash: stash.clone(),
            },
        );
        // Keep [`AttestorsCount`] in lock-step with [`Attestors`] so
        // [`attestor_list_has_space`] stays O(1). `saturating_add` guards against
        // a pathological counter already at `u32::MAX`; the `ensure!` above bounds
        // actual growth to `MaxAttestors`.
        AttestorsCount::<T>::mutate(chain_key, |count| {
            *count = count.saturating_add(1);
        });

        // Make sure the stash can pay for the registration
        let stash_balance = Self::get_free_balance(&stash);
        ensure!(
            stash_balance >= Self::min_bond_requirement(chain_key),
            Error::<T>::InsufficientBalance
        );

        // Get some amount to fund the attestor
        let amount: BalanceOf<T> = ONE_TENTH_CTC.into();

        // Fund the attestor key
        T::Currency::transfer(
            &stash,
            &attestor_id,
            amount,
            frame_support::traits::ExistenceRequirement::KeepAlive,
        )?;

        // Create a new ledger for the attestor
        // With minimum bond requirement
        let ledger: AttestorLedger<T> =
            AttestorLedger::new(stash.clone(), Self::min_bond_requirement(chain_key));

        // If bond fails, it means it's already bonded and there is already an attestor(s) registerd by this stash
        // In this case, we just bond extra to the stash
        if ledger.bond().is_err() {
            Self::bond_extra(chain_key, &stash)?;
        } else {
            // Would fail if account has no provider.
            frame_system::Pallet::<T>::inc_consumers(&stash)?;

            // Emit event
            Self::deposit_event(Event::<T>::Bonded {
                stash,
                amount: Self::min_bond_requirement(chain_key),
            });
        }

        // Emit event
        Self::deposit_event(Event::<T>::AttestorRegistered(chain_key, attestor_id));

        Ok(())
    }

    fn current_era() -> u32 {
        T::Staking::current_era()
    }

    // Deregister an attestor and start unlocking the funds.
    // Attstor needs to call `chill` first before the stash can deregister the attestor
    // Remove that attestor and emit an event
    pub fn remove_attestor_and_emit_event(
        chain_key: ChainKey,
        stash: T::AccountId,
        attestor_id: T::AccountId,
    ) -> DispatchResult {
        let attestor =
            Attestors::<T>::get(chain_key, &attestor_id).ok_or(Error::<T>::AddressNotAttestor)?;

        // Only remove your own attestor
        ensure!(attestor.stash == stash, Error::<T>::NotYourAttestor);

        // Ensure the attestor is idle
        // Attestor needs to call `chill` first
        ensure!(
            attestor.status == AttestorStatus::Idle,
            Error::<T>::AttestorNotIdle
        );

        // Get the min bond requirement for the attestor
        let bond = Self::min_bond_requirement(chain_key);

        let mut ledger = Self::ledger(&stash).ok_or(Error::<T>::NotStash)?;
        // Value is the minimum of the bond and the active amount
        let mut value = bond.min(ledger.active);

        ensure!(
            ledger.unlocking.len() < T::MaxUnlockingChunks::get() as usize,
            Error::<T>::NoMoreChunks,
        );

        // Aligns with the unlock chunk era used below (unbond completes at or after this era).
        let purge_at_era = Self::current_era().defensive_saturating_add(T::BondingDuration::get());

        if !value.is_zero() {
            // Decrease the active amount
            ledger.active -= value;

            // Avoid there being a dust balance left in the staking system.
            if ledger.active < existential_deposit::<T>() {
                value += ledger.active;
                ledger.active = Zero::zero();
            }

            // Note: in case there is no current era it is fine to bond one era more.
            if let Some(chunk) = ledger
                .unlocking
                .last_mut()
                .filter(|chunk| chunk.era == purge_at_era)
            {
                // To keep the chunk count down, we only keep one chunk per era. Since
                // `unlocking` is a FiFo queue, if a chunk exists for `era` we know that it will
                // be the last one.
                chunk.value = chunk.value.defensive_saturating_add(value)
            } else {
                ledger
                    .unlocking
                    .try_push(UnlockChunk {
                        value,
                        era: purge_at_era,
                    })
                    .map_err(|_| Error::<T>::NoMoreChunks)?;
            };

            // Update the ledger
            ledger.update()?;

            Self::deposit_event(Event::<T>::Unbonded {
                stash: stash.clone(),
                amount: value,
            });
        }

        if let Some(bls_public_key) = attestor.bls_public_key {
            RetiredAttestorBlsKeys::<T>::insert(
                chain_key,
                &attestor_id,
                RetiredAttestorBlsKeyEntry {
                    bls_public_key,
                    purge_at_era,
                    stash: stash.clone(),
                },
            );
            RetiredAttestorKeysByStash::<T>::mutate(&stash, |vec| {
                vec.try_push((chain_key, attestor_id.clone()))
                    .map_err(|_| Error::<T>::RetiredAttestorPendingFull)
            })?;
        }

        Self::remove_active_attestor_from_set(chain_key, &attestor_id);

        // Remove the attestor (BLS key may remain in [`RetiredAttestorBlsKeys`] until unbond ends)
        Attestors::<T>::remove(chain_key, &attestor_id);
        // Keep [`AttestorsCount`] in lock-step with [`Attestors`]. `saturating_sub`
        // defends against drift (e.g. a pre-migration chain whose count was not
        // populated); the actual decrement pairs with the insert above.
        AttestorsCount::<T>::mutate(chain_key, |count| {
            *count = count.saturating_sub(1);
        });

        Self::deposit_event(Event::<T>::AttestorUnregistered(chain_key, attestor_id));

        Ok(())
    }

    pub(crate) fn do_commit_attestation(
        attestation: SignedAttestation<T::Hash, T::AccountId>,
    ) -> DispatchResult {
        let chain_key = attestation.chain_key();

        // Validate the attestation
        Self::validate_attestation(chain_key, &attestation)?;

        // Store the attestation
        let digest = attestation.digest();
        let header_number = attestation.header_number();
        Attestations::<T>::insert(chain_key, digest, &attestation);

        // Update last digest
        LastDigest::<T>::mutate(chain_key, |last_digest| {
            if last_digest
                .as_ref()
                .is_none_or(|(h, ..)| header_number > *h)
            {
                *last_digest = Some((header_number, digest));
            }
        });

        // Emit event
        Self::deposit_event(Event::<T>::BlockAttested(chain_key, header_number, digest));

        if Checkpoints::<T>::iter_prefix(chain_key).next().is_none() {
            let genesis_block_number = AttestationChainGenesisBlockNumber::<T>::get(chain_key);
            ensure!(
                genesis_block_number == header_number,
                Error::<T>::InvalidAttestationBlockNumber
            );

            // Very first attestation should have a corresponding checkpoint
            // even though it doesn't condense any prior attestations.
            let checkpoint = AttestationCheckpoint {
                block_number: header_number,
                digest,
            };

            Self::deposit_event(Event::<T>::CheckpointReached(chain_key, checkpoint.clone()));

            Checkpoints::<T>::insert(chain_key, header_number, checkpoint.digest);
            CheckpointBuckets::<T>::insert(
                (
                    chain_key,
                    Self::compute_block_index_for(header_number),
                    header_number,
                ),
                (),
            );
            LastCheckpoint::<T>::insert(chain_key, &checkpoint);
            // Add first attestation to removal queue, since first checkpoint was already created
            AttestationRemovalQueues::<T>::mutate(chain_key, |queue| {
                queue.push_back(digest);
            });
            return Ok(());
        }

        // When catching up (large continuity proof spanning 2+ checkpoint boundaries), create
        // checkpoints directly from the proof. Otherwise use the legacy queue-based flow.
        if Self::create_checkpoints_from_continuity_proof(chain_key, &attestation, digest)? {
            return Ok(());
        }

        let mut queue = CheckpointingQueues::<T>::get(chain_key);
        queue.push_back(digest);

        // Make checkpoint if necessary (legacy path for queue-based checkpointing).
        // The extrinsic didn't fail even if checkpointing failed. We want
        // to keep the new attestation rather than removing it from storage
        // via extrinsic rollback in the case of checkpointing failure.
        if let Err(e) = Self::try_make_checkpoint(&mut queue, chain_key, header_number) {
            log::error!("Error: {e:?}");
        }
        CheckpointingQueues::<T>::insert(chain_key, queue);

        Ok(())
    }

    pub fn start_attesting(
        chain_key: ChainKey,
        attestor_id: T::AccountId,
        bls_public_key: BlsPublicKey,
        proof_of_possession: BlsSignature,
    ) -> DispatchResult {
        let mut attestor =
            Attestors::<T>::get(chain_key, &attestor_id).ok_or(Error::<T>::AddressNotAttestor)?;

        // Check if attestor has already been registered and if they're not already waiting/active
        if attestor.bls_public_key.is_some() {
            ensure!(
                attestor.status == AttestorStatus::Idle,
                Error::<T>::AttestorNotIdle
            );
        }

        // Verify proof of possession
        let public_key = PublicKey::from_bytes(&bls_public_key[..])
            .map_err(|_| Error::<T>::InvalidBlsPublicKey)?;

        let signature = Signature::from_bytes(&proof_of_possession[..])
            .map_err(|_| Error::<T>::InvalidBlsSignature)?;

        ensure!(
            bls_signatures::verify(
                &signature,
                &[bls_signatures::hash(bls_public_key[..].into())],
                &[public_key]
            ),
            Error::<T>::InvalidProofOfPossession
        );

        // Enforce BLS pubkey uniqueness per chain. The aggregation quorum is meaningful only
        // when each contributing key was produced by an independent private key — without
        // this gate, multiple controller accounts can register the same BLS pubkey (each
        // passing PoP independently) and a single signer can satisfy threshold-of-N.
        //
        // We also reject keys currently sitting in [`RetiredAttestorBlsKeys`] for this
        // chain: those keys remain verifiable in pending aggregated attestations until the
        // unbond delay elapses, so reusing them under a fresh controller would still allow
        // an aliased-key attack against in-flight quorums.
        match BlsKeyOwner::<T>::get(chain_key, bls_public_key) {
            None => {}
            Some(existing_owner) if existing_owner == attestor_id => {
                // Re-asserting the same key for the same controller (idempotent path,
                // e.g. re-attesting after a chill with the same key) — allow.
            }
            Some(_) => return Err(Error::<T>::BlsKeyAlreadyRegistered.into()),
        }

        // Key rotation: an idle attestor that previously had a different BLS key must
        // release its old `BlsKeyOwner` claim before taking the new one. Skipped when the
        // old and new keys are identical (handled by the match above already returning Ok).
        if let Some(old_key) = attestor.bls_public_key {
            if old_key != bls_public_key {
                BlsKeyOwner::<T>::remove(chain_key, old_key);
            }
        }

        BlsKeyOwner::<T>::insert(chain_key, bls_public_key, &attestor_id);

        // Set status to Waiting until next epoch rotation
        attestor.status = AttestorStatus::Waiting;
        attestor.bls_public_key = Some(bls_public_key);
        Attestors::<T>::insert(chain_key, &attestor_id, attestor);

        Self::deposit_event(Event::<T>::AttestorActivated(
            chain_key,
            attestor_id,
            bls_public_key,
        ));

        Ok(())
    }

    /// Voluntary chill for an active attestor: stay in the current epoch committee until the next
    /// election finalizes the transition to [`AttestorStatus::Idle`] (same era boundary semantics
    /// as [`AttestorStatus::Waiting`] → active).
    pub(crate) fn schedule_voluntary_chill(chain_key: ChainKey, attestor_id: T::AccountId) {
        Attestors::<T>::mutate(chain_key, &attestor_id, |maybe_attestor| {
            if let Some(attestor) = maybe_attestor {
                attestor.status = AttestorStatus::Leaving;
            }
        });
    }

    /// Immediately set an attestor to idle, remove them from the active set, and emit
    /// [`Event::AttestorChilled`]. Used for operator kicks, chain cleanup, and chilling from
    /// [`AttestorStatus::Waiting`] (cancel pending activation).
    pub(crate) fn do_chill_attestor_immediate(chain_key: ChainKey, attestor_id: T::AccountId) {
        Attestors::<T>::mutate(chain_key, &attestor_id, |maybe_attestor| {
            if let Some(attestor) = maybe_attestor {
                attestor.status = AttestorStatus::Idle;
            }
        });

        Self::remove_active_attestor_from_set(chain_key, &attestor_id);

        Self::deposit_event(Event::<T>::AttestorChilled(chain_key, attestor_id));
    }

    pub(super) fn do_withdraw_unbonded(stash: &T::AccountId) -> DispatchResult {
        let mut ledger = Self::ledger(stash).ok_or(Error::<T>::NotStash)?;

        let (stash, old_total) = (ledger.stash.clone(), ledger.total_staked);

        let current_era = Self::current_era();
        if current_era > 0 {
            ledger = ledger.consolidate_unlocked(current_era)
        }
        Self::purge_retired_bls_keys_for_stash(&stash, current_era);
        let new_total = ledger.total_staked;

        let ed = T::Currency::minimum_balance();
        if ledger.unlocking.is_empty() && (ledger.active < ed || ledger.active.is_zero()) {
            // This account must have called `unbond()` with some value that caused the active
            // portion to fall below existential deposit + will have no more unlocking chunks
            // left. We can now safely remove all staking-related information.
            Self::kill_stash(&ledger.stash)?;
        } else {
            // This was the consequence of a partial unbond. just update the ledger and move on.
            ledger.update()?;
        };

        // `old_total` should never be less than the new total because
        // `consolidate_unlocked` strictly subtracts balance.
        if new_total < old_total {
            // Already checked that this won't overflow by entry condition.
            let value = old_total.defensive_saturating_sub(new_total);
            Self::deposit_event(Event::<T>::Withdrawn {
                stash,
                amount: value,
            });
        }

        Ok(())
    }
}

/// NON-CALL FUNCTIONS ///
impl<T: Config> Pallet<T> {
    pub(super) fn bond_extra(chain_key: ChainKey, stash: &T::AccountId) -> DispatchResult {
        let bond = Self::min_bond_requirement(chain_key);

        let mut ledger = Self::ledger(stash.clone()).ok_or(Error::<T>::NotStash)?;

        let extra = bond.min(
            T::Currency::free_balance(stash)
                .checked_sub(&ledger.total_staked)
                .ok_or(ArithmeticError::Overflow)?,
        );

        // Update total staked and active amount
        ledger.total_staked = ledger
            .total_staked
            .checked_add(&extra)
            .ok_or(ArithmeticError::Overflow)?;
        ledger.active = ledger
            .active
            .checked_add(&extra)
            .ok_or(ArithmeticError::Overflow)?;

        // NOTE: ledger must be updated prior to calling `Self::weight_of`.
        ledger.update()?;

        Self::deposit_event(Event::<T>::Bonded {
            stash: stash.clone(),
            amount: extra,
        });

        Ok(())
    }

    /// Remove all associated data of a stash account from the staking system.
    ///
    /// Assumes storage is upgraded before calling.
    ///
    /// This is called:
    /// - after a `withdraw_unbonded()` call that frees all of a stash's bonded balance.
    /// - through `reap_stash()` if the balance has fallen to zero (through slashing).
    pub(crate) fn kill_stash(stash: &T::AccountId) -> DispatchResult {
        // removes controller from `Bonded` and staking ledger from `Ledger`, as well as reward
        // setting of the stash in `Payee`.
        AttestorLedger::<T>::kill(stash)?;

        frame_system::Pallet::<T>::dec_consumers(stash);

        Ok(())
    }

    /// Start a new election for the given epoch
    /// This will select the active attestors for the given epoch
    /// All attestors with `Active` status will be selected
    /// Attestors with `Waiting` status will be selected based on the election policy
    /// Attestors with `Leaving` status become `Idle` and are not selected (deferred voluntary chill)
    pub fn do_start_election(epoch: u64, _randomness: Randomness) -> DispatchResult {
        let supported_chains = T::SupportedChains::supported_chains();

        for chain_key in supported_chains {
            let chain_election_policy = ChainElectionPolicy::<T>::get(chain_key);
            let prefix: Vec<_> = Attestors::<T>::iter_prefix(chain_key).collect();
            let prefix_len = prefix.len();

            let attestors = prefix
                .into_iter()
                .filter_map(|(account, mut attestor)| {
                    match attestor.status {
                        AttestorStatus::Active => Some(account),
                        AttestorStatus::Waiting => {
                            match chain_election_policy {
                                AttestorElectionPolicy::OpenToAny => {
                                    // Transition from Waiting to Active
                                    attestor.status = AttestorStatus::Active;
                                    Attestors::<T>::insert(chain_key, &account, attestor);
                                    Some(account)
                                }
                                AttestorElectionPolicy::AuthorizedOnly => {
                                    // If the attestor is not authorized, skip them
                                    if !AuthorizedAttestors::<T>::contains_key(chain_key, &account)
                                    {
                                        debug!(
                                            "Skipping attestor {account:?} for chain {chain_key} as they are not authorized",
                                        );
                                        None
                                    } else {
                                        // Transition from Waiting to Active
                                        attestor.status = AttestorStatus::Active;
                                        Attestors::<T>::insert(chain_key, &account, attestor);
                                        Some(account)
                                    }
                                },
                                AttestorElectionPolicy::DeniedToAll => {
                                    debug!(
                                        "Skipping attestor {account:?} for chain {chain_key} as election policy is DeniedToAll",
                                    );
                                    None
                                }
                            }
                        }
                        AttestorStatus::Idle => None,
                        AttestorStatus::Leaving => {
                            attestor.status = AttestorStatus::Idle;
                            Attestors::<T>::insert(chain_key, &account, attestor);
                            Self::deposit_event(Event::<T>::AttestorChilled(chain_key, account.clone()));
                            None
                        }
                    }
                })
                .collect::<Vec<_>>();

            // We still need an event if the number of attestors went from non-zero to zero
            if attestors.is_empty() && prefix_len == 0 {
                debug!("No active attestors for chain {chain_key}");
                continue;
            }

            ActiveAttestors::<T>::insert(chain_key, &attestors);

            Self::deposit_event(Event::<T>::AttestorsElected {
                epoch,
                chain_key,
                attestors,
            });
        }

        Ok(())
    }

    /// Get the locked balance of an account
    /// This is the total amount of balance that is locked by this module
    pub fn get_locked_balance(account_id: &T::AccountId) -> BalanceOf<T> {
        let balance_locks = pallet_balances::Pallet::<T>::locks(account_id);

        let mut locked_balance = BalanceOf::<T>::zero();
        // loop over balance accumulate locked balance
        for lock in balance_locks {
            if lock.id == crate::ledger::BOND_LOCK_ID {
                locked_balance +=
                    BalanceOf::<T>::saturated_from(lock.amount.saturated_into::<u128>());
            }
        }

        locked_balance
    }

    /// Get the free balance of an account
    /// This is the total balance minus the locked balance
    pub fn get_free_balance(account_id: &T::AccountId) -> BalanceOf<T> {
        // This is the existential deposit
        let min_b = T::Currency::minimum_balance();
        // Free balance of the account
        let free_b = T::Currency::free_balance(account_id);
        // Locked balance of the account
        let locked_balance = Self::get_locked_balance(account_id);

        // Free balance is the total balance minus the minimum balance and the locked balance
        free_b.saturating_sub(min_b).saturating_sub(locked_balance)
    }

    pub fn apply_interval_updates() {
        PendingAttestationInterval::<T>::iter().for_each(
            |(chain_key, new_attestation_interval)| {
                ChainAttestationInterval::<T>::set(chain_key, new_attestation_interval);

                Self::deposit_event(Event::<T>::AttestationIntervalChanged(
                    chain_key,
                    new_attestation_interval,
                ));
            },
        );

        PendingTargetSampleSize::<T>::iter().for_each(|(chain_key, new_target_sample_size)| {
            TargetSampleSize::<T>::set(chain_key, new_target_sample_size);

            Self::deposit_event(Event::<T>::TargetSampleSizeChanged(
                chain_key,
                new_target_sample_size,
            ));
        });

        PendingMaxCatchup::<T>::iter().for_each(|(chain_key, new_max_catchup)| {
            MaxCatchup::<T>::set(chain_key, new_max_catchup);

            Self::deposit_event(Event::<T>::MaxCatchupChanged(chain_key, new_max_catchup));
        });

        // Clear PendingAttestationInterval, PendingTargetSampleSize & PendingMaxCatchup
        let num_supported_chains = T::SupportedChains::supported_chains().len();
        let _ = PendingAttestationInterval::<T>::clear(num_supported_chains as u32, None);
        let _ = PendingTargetSampleSize::<T>::clear(num_supported_chains as u32, None);
        let _ = PendingMaxCatchup::<T>::clear(num_supported_chains as u32, None);
    }

    pub(crate) fn chill_all_attestors_for_chain(chain_key: ChainKey) {
        let attestor_ids = Attestors::<T>::iter_key_prefix(chain_key);
        for attestor_id in attestor_ids {
            Self::do_chill_attestor_immediate(chain_key, attestor_id);
        }
    }

    // Remove address as invulnerable and attestor
    pub(crate) fn remove_invulnerable_and_emit_event(
        chain_key: ChainKey,
        address: T::AccountId,
    ) -> DispatchResult {
        // Remove from invulnerables
        Invulnerables::<T>::remove(chain_key, &address);
        Self::deposit_event(Event::<T>::InvulnerableUnregistered(
            chain_key,
            address.clone(),
        ));

        Ok(())
    }

    pub fn working_set_size(chain_key: ChainKey) -> u32 {
        ActiveAttestors::<T>::get(chain_key).len() as u32
    }

    pub fn is_attestor(chain_key: ChainKey, address: &T::AccountId) -> bool {
        let active_attestors = ActiveAttestors::<T>::get(chain_key);
        active_attestors.contains(address)
    }

    pub fn attestor_status(chain_key: ChainKey, address: &T::AccountId) -> Option<AttestorStatus> {
        Attestors::<T>::get(chain_key, address).map(|attestor| attestor.status)
    }

    pub fn address_is_not_attestor(chain_key: ChainKey, address: &T::AccountId) -> bool {
        !Self::is_attestor(chain_key, address)
    }

    pub fn attestor_is_registered(chain_key: ChainKey, address: &T::AccountId) -> bool {
        Attestors::<T>::contains_key(chain_key, address)
    }

    pub fn last_digest(chain_key: ChainKey) -> Option<Digest> {
        LastDigest::<T>::get(chain_key)
            .map(|(_, digest)| digest)
            .or_else(|| LastCheckpoint::<T>::get(chain_key).map(|c| c.digest))
    }

    pub fn contains_digest(chain_key: ChainKey, digest: Digest, block_number: u64) -> bool {
        Attestations::<T>::contains_key(chain_key, digest)
            || Checkpoints::<T>::get(chain_key, block_number) == Some(digest)
    }

    pub fn attestor_bls_pubkey(
        chain_key: ChainKey,
        address: &T::AccountId,
    ) -> Option<BlsPublicKey> {
        Attestors::<T>::get(chain_key, address)
            .map(|attestor| attestor.bls_public_key)
            .unwrap_or_default()
    }

    pub fn attestor_list_has_space(chain_key: ChainKey) -> bool {
        AttestorsCount::<T>::get(chain_key) < MaxAttestors::<T>::get(chain_key)
    }

    pub fn get(
        chain_key: ChainKey,
        digest: Digest,
    ) -> Option<SignedAttestation<T::Hash, T::AccountId>> {
        Attestations::<T>::get(chain_key, digest)
    }

    pub(crate) fn vulnerable_list_has_space(chain_key: ChainKey) -> bool {
        let count = Invulnerables::<T>::iter_prefix_values(chain_key)
            .collect::<Vec<_>>()
            .len() as u32;
        count < MaxInvulnerables::<T>::get(chain_key)
    }

    /// Insert address as attestor & invulnerable
    pub(crate) fn try_insert_invulnerable_and_emit_event(
        chain_key: ChainKey,
        address: &T::AccountId,
    ) -> DispatchResult {
        ensure!(
            Self::vulnerable_list_has_space(chain_key),
            Error::<T>::InvulnerableListFull
        );

        Invulnerables::<T>::insert(chain_key, address, true);
        Self::deposit_event(Event::<T>::InvulnerableRegistered(
            chain_key,
            address.clone(),
        ));
        Ok(())
    }

    pub(crate) fn address_is_invulnerable(chain_key: ChainKey, address: &T::AccountId) -> bool {
        Invulnerables::<T>::contains_key(chain_key, address)
    }

    /// Validate the attestation
    /// This checks if the attestation is valid
    /// The attestation is valid if the BLS signature is valid
    /// Is valid if the attestation is not a duplicate
    /// Is valid if the chain is supported
    pub(crate) fn validate_attestation(
        chain_key: ChainKey,
        attestation: &SignedAttestation<T::Hash, T::AccountId>,
    ) -> DispatchResult {
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );

        if Self::check_duplicate(attestation) {
            return Err(Error::<T>::AttestationExists.into());
        }

        // Attestor eligibility validation
        let active_attestors = ActiveAttestors::<T>::get(chain_key)
            .into_iter()
            .collect::<BTreeSet<_>>(); // or HashSet if std is available

        let eligible_attestors: BTreeSet<T::AccountId> = attestation
            .attestors
            .iter()
            .filter(|attestor| active_attestors.contains(attestor))
            .cloned()
            .collect();

        // Threshold validation
        let target_sample_size = Self::target_sample_size(chain_key);
        let threshold = calculate_threshold(target_sample_size);
        ensure!(
            eligible_attestors.len() as u32 >= threshold,
            Error::<T>::MajorityNotReached
        );

        // Signature verification
        let agg_signature = Self::extract_agg_signature(&attestation.signature)?;
        let attestor_public_keys =
            Self::gather_attestor_public_keys(attestation.chain_key(), &attestation.attestors)?;

        // Validation-time dedup: defense-in-depth against duplicate BLS keys appearing in
        // the attestor set (pre-`v3` storage, migration drift, retired-key edge cases).
        // BLS aggregation is linear, so aggregating the same key `k` `n` times yields `n*k`
        // and a single signer holding `s` (with `s*G = k`) can satisfy the quorum by having
        // their signature counted `n` times. We enforce that the number of *distinct* BLS
        // public keys clears the threshold, and we aggregate over the deduped set so the
        // aggregate verification check itself does not accept replayed contributions.
        let mut deduped_public_keys: Vec<PublicKey> =
            Vec::with_capacity(attestor_public_keys.len());
        let mut seen_keys: BTreeSet<BlsPublicKey> = BTreeSet::new();
        for pk in attestor_public_keys.iter() {
            let pk_bytes: BlsPublicKey = pk.as_bytes()[..].try_into().map_err(|_| {
                log::error!("Unexpected BLS public key encoding length");
                Error::<T>::InvalidBlsPublicKey
            })?;
            if seen_keys.insert(pk_bytes) {
                deduped_public_keys.push(*pk);
            }
        }
        ensure!(
            deduped_public_keys.len() as u32 >= threshold,
            Error::<T>::InsufficientUniqueSigners
        );

        let aggregated_public_key =
            aggregate_public_keys(&deduped_public_keys[..]).map_err(|_| {
                log::error!("Failed to aggregate public keys");
                Error::<T>::InvalidBlsSignature
            })?;

        let message = &attestation.attestation.serialize()[..];

        // Verify the aggregated signature
        Self::verify_agg_signature(&agg_signature, message, aggregated_public_key)?;
        log::debug!("Attestation signature is valid");

        // Continuity validation
        Self::validate_attestation_continuity(attestation)?;
        log::debug!("Attestation continuity is valid");

        Ok(())
    }

    /// When enough blocks have been attested since the last checkpoint,
    /// create a new checkpoint condensing prior attestations.
    /// The checkpoint will be created at the next block that is a multiple of
    /// chain_attestation_interval * attestation_checkpoint_interval after the last checkpoint.
    #[transactional]
    pub(crate) fn try_make_checkpoint(
        queue: &mut VecDeque<Digest>,
        chain_key: ChainKey,
        attestation_header: u64,
    ) -> DispatchResult {
        // We get the last checkpoint header number, or we error out if there is none
        // since the first checkpoint should have already been created before calling this function
        let last_checkpoint_header = LastCheckpoint::<T>::get(chain_key)
            .ok_or(Error::<T>::LastCheckpointEmpty)?
            .block_number;

        // Compute the checkpoint width
        let attestation_interval = Self::chain_attestation_interval(chain_key);
        let checkpoint_interval = Self::attestation_checkpoint_interval(chain_key);
        let checkpoint_width = attestation_interval.saturating_mul(checkpoint_interval as u64);
        ensure!(checkpoint_width > 0, Error::<T>::CheckpointWidthIsZero);

        // Check if the current attestation span is enough to create a new checkpoint
        let attestation_block_span = attestation_header.saturating_sub(last_checkpoint_header);
        if attestation_block_span < (checkpoint_width * 2) + 1 {
            return Ok(());
        }

        // Compute the header block number for the new checkpoint
        let target_block = {
            // First we compute the next checkpoint block after the last checkpoint
            let next_checkpoint_block = last_checkpoint_header.saturating_add(checkpoint_width);

            // Then we round it down to the nearest multiple of checkpoint_width
            next_checkpoint_block - (next_checkpoint_block % checkpoint_width)
        };

        // Queue used to time the removal of attestations for some duration after checkpoint creation
        let mut attestation_removal_queue: VecDeque<Digest> =
            AttestationRemovalQueues::<T>::get(chain_key);

        // Because checkpointing queue storage is written to after this function
        // returns, it isn't covered by the #[transactional] macro and must be manually
        // rolled back. Rollback replays pops with `push_front`; iterating the rollback
        // vec in reverse keeps oldest-at-front order (forward iteration would reverse it).
        let mut checkpointing_rollback: Vec<Digest> = Vec::new();

        let new_checkpoint = loop {
            let attestation_digest: Digest = match queue.pop_front() {
                Some(digest) => digest,
                None => {
                    for digest in checkpointing_rollback.into_iter().rev() {
                        queue.push_front(digest);
                    }
                    return Err(Error::<T>::CheckpointingQueueDrained.into());
                }
            };
            checkpointing_rollback.push(attestation_digest);

            let attestation = match Attestations::<T>::get(chain_key, attestation_digest) {
                Some(attestation) => attestation,
                None => {
                    for digest in checkpointing_rollback.into_iter().rev() {
                        queue.push_front(digest);
                    }
                    return Err(Error::<T>::AttestationNotFound.into());
                }
            };

            match attestation.header_number().cmp(&target_block) {
                sp_std::cmp::Ordering::Less => {
                    // If the header is smaller than the target block, we need to keep condensing into, at least,
                    // the next attestation
                    attestation_removal_queue.push_back(attestation_digest);
                }
                sp_std::cmp::Ordering::Equal => {
                    // If the attestation header matches the target block, we can both use it and mark it for removal
                    attestation_removal_queue.push_back(attestation_digest);

                    break AttestationCheckpoint {
                        block_number: attestation.header_number(),
                        digest: attestation.digest(),
                    };
                }
                sp_std::cmp::Ordering::Greater => {
                    // If the attestation header is greater than the target block,
                    // we need to find the block within its continuity proof
                    // that matches the target block we want to condense to.
                    // USC-001: Compute digest from roots rather than trusting proof.digest.
                    let start_block_number = attestation
                        .continuity_proof
                        .start_block_number(attestation.header_number());
                    let maybe_digest = attestation
                        .continuity_proof
                        .digest_for_block(start_block_number, target_block);

                    match maybe_digest {
                        Some(digest) => {
                            // We found the target block within this attestation's continuity proof
                            // we build the checkpoint and return the attestation digest back to the queue
                            // since it will be used in the next checkpointing round
                            queue.push_front(attestation_digest);

                            break AttestationCheckpoint {
                                block_number: target_block,
                                digest,
                            };
                        }
                        None => {
                            // We couldn't find the target block within this attestation's continuity proof
                            for digest in checkpointing_rollback.into_iter().rev() {
                                queue.push_front(digest);
                            }
                            return Err(Error::<T>::CheckpointTargetNotFound.into());
                        }
                    }
                }
            }
        };

        Self::deposit_event(Event::<T>::CheckpointReached(
            chain_key,
            new_checkpoint.clone(),
        ));

        Checkpoints::<T>::insert(
            chain_key,
            new_checkpoint.block_number,
            new_checkpoint.digest,
        );
        CheckpointBuckets::<T>::insert(
            (
                chain_key,
                Self::compute_block_index_for(new_checkpoint.block_number),
                new_checkpoint.block_number,
            ),
            (),
        );
        LastCheckpoint::<T>::insert(chain_key, &new_checkpoint);

        Self::remove_attestations(chain_key, attestation_removal_queue)?;

        Ok(())
    }

    /// Create checkpoints dynamically from an attestation's continuity proof when catching up
    /// (proof spans 2+ checkpoint intervals). Otherwise no-op so the legacy queue-based flow
    /// handles normal attestation cadence.
    ///
    /// Returns `Ok(true)` when checkpoints were created and the digest was added to
    /// AttestationRemovalQueues (caller must remove it from CheckpointingQueues to avoid duplicate).
    #[transactional]
    pub(crate) fn create_checkpoints_from_continuity_proof(
        chain_key: ChainKey,
        attestation: &SignedAttestation<T::Hash, T::AccountId>,
        attestation_digest: Digest,
    ) -> Result<bool, DispatchError> {
        if attestation.continuity_proof.is_empty() {
            return Ok(false);
        }

        let last_checkpoint =
            LastCheckpoint::<T>::get(chain_key).ok_or(Error::<T>::LastCheckpointEmpty)?;

        let attestation_interval = Self::chain_attestation_interval(chain_key);
        let checkpoint_interval = Self::attestation_checkpoint_interval(chain_key);
        let checkpoint_width = attestation_interval.saturating_mul(checkpoint_interval as u64);
        ensure!(checkpoint_width > 0, Error::<T>::CheckpointWidthIsZero);

        // Only when catching up: proof spans 2+ checkpoint intervals
        let proof_len = attestation.continuity_proof.len();
        if proof_len < 2 * checkpoint_width as usize {
            return Ok(false);
        }

        // Build map of block_number -> digest from continuity proof.
        // USC-001: Compute digests from roots rather than trusting proof.digest values.
        // Include the attestation block itself so boundaries at header_number are not missed.
        let start_block_number = attestation
            .continuity_proof
            .start_block_number(attestation.header_number());
        let mut block_digests: BTreeMap<u64, Digest> = attestation
            .continuity_proof
            .block_digests(start_block_number);
        block_digests.insert(attestation.header_number(), attestation_digest);

        // Also include digests from queued attestations that precede the current proof.
        // When the attestation chain is not aligned to a checkpoint boundary, the
        // CheckpointingQueues may contain attestations covering blocks between
        // the last checkpoint and the start of this proof. We need their digests
        // to create checkpoints at boundaries that fall in that gap.
        let mut checkpointing_queue = CheckpointingQueues::<T>::get(chain_key);
        for queued_digest in checkpointing_queue.iter() {
            if let Some(queued_att) = Attestations::<T>::get(chain_key, queued_digest) {
                // Add the attestation header block itself
                block_digests
                    .entry(queued_att.header_number())
                    .or_insert(queued_att.digest());
                // Add all blocks from its continuity proof (digests computed from roots)
                let queued_start = queued_att
                    .continuity_proof
                    .start_block_number(queued_att.header_number());
                for (block_number, digest) in
                    queued_att.continuity_proof.block_digests(queued_start)
                {
                    block_digests.entry(block_number).or_insert(digest);
                }
            }
        }

        let head_block = attestation.header_number();

        let mut last_checkpoint_block = last_checkpoint.block_number;
        let mut attestation_removal_queue: VecDeque<Digest> =
            AttestationRemovalQueues::<T>::get(chain_key);

        // Create checkpoints for each boundary block in the continuity proof.
        // Target calculation mirrors the legacy path: round down to nearest global
        // multiple of checkpoint_width to keep checkpoint placement consistent.
        //
        // Also match `try_make_checkpoint`: only advance when the attestation head is at least
        // `2 * checkpoint_width + 1` blocks past the last checkpoint (same span rule).
        loop {
            let next = last_checkpoint_block.saturating_add(checkpoint_width);
            let target_block = next.saturating_sub(next % checkpoint_width);
            if target_block > head_block {
                break;
            }

            let Some(digest) = block_digests.get(&target_block) else {
                log::error!(
                    "Continuity proof missing expected checkpoint boundary block {target_block} for chain {chain_key:?}"
                );
                return Err(Error::<T>::CheckpointCreationError.into());
            };

            let new_checkpoint = AttestationCheckpoint {
                block_number: target_block,
                digest: *digest,
            };

            Self::deposit_event(Event::<T>::CheckpointReached(
                chain_key,
                new_checkpoint.clone(),
            ));

            Checkpoints::<T>::insert(
                chain_key,
                new_checkpoint.block_number,
                new_checkpoint.digest,
            );
            CheckpointBuckets::<T>::insert(
                (
                    chain_key,
                    Self::compute_block_index_for(new_checkpoint.block_number),
                    new_checkpoint.block_number,
                ),
                (),
            );
            LastCheckpoint::<T>::insert(chain_key, &new_checkpoint);
            last_checkpoint_block = target_block;
        }

        // Add attestation to removal queue once if we created any checkpoints from it
        if last_checkpoint_block != last_checkpoint.block_number {
            // Add all queued attestation digests to removal queue — they've been
            // consumed by the checkpoint creation above.
            for queued_digest in checkpointing_queue.drain(..) {
                attestation_removal_queue.push_back(queued_digest);
            }
            attestation_removal_queue.push_back(attestation_digest);
            // remove_attestations writes the queue to storage, no need to insert beforehand
            Self::remove_attestations(chain_key, attestation_removal_queue)?;
            // Clear any stale entries in CheckpointingQueues — the catch-up path has
            // advanced LastCheckpoint past them, so they'd be invalid if processed later.
            CheckpointingQueues::<T>::remove(chain_key);
            return Ok(true);
        }

        Ok(false)
    }

    #[transactional]
    pub(crate) fn do_import_checkpoints(
        chain_key: ChainKey,
        checkpoints: BoundedVec<AttestationCheckpoint, T::MaxCheckpointsImportedPerCall>,
    ) -> DispatchResult {
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );

        let has_attestations = LastDigest::<T>::get(chain_key).is_some();

        let maybe_last_checkpoint: Option<AttestationCheckpoint> =
            LastCheckpoint::<T>::get(chain_key);
        let initial_block_number = maybe_last_checkpoint
            .as_ref()
            .map(|c| c.block_number)
            .unwrap_or(0);

        let mut maybe_new_last_checkpoint = maybe_last_checkpoint.clone();

        // We go through all checkpoints and we only import the ones that are not already in storage and that are older than the latest checkpoint/attestation,
        // since newer ones are going to be created by the normal checkpointing flow and we want to avoid conflicts between the two flows.
        for checkpoint in checkpoints {
            if Checkpoints::<T>::contains_key(chain_key, checkpoint.block_number) {
                continue;
            }

            match (has_attestations, &maybe_last_checkpoint) {
                (_, Some(last_checkpoint))
                    if checkpoint.block_number >= last_checkpoint.block_number =>
                {
                    // If we have both attestations and checkpoints, we only allow the import of checkpoints that are older than the latest checkpoint
                    log::debug!(
                        "Skipping import of checkpoint at block number {} for chain {:?} since it's newer than the latest checkpoint at block number {}",
                        checkpoint.block_number,
                        chain_key,
                        last_checkpoint.block_number
                    );
                    continue;
                }
                (true, None) => {
                    // If we only have attestations, and no checkpoints, we don't allow the import of checkpoints at all.
                    // Caller will have to wait until the normal checkpointing flow creates the first checkpoint.
                    return Err(Error::<T>::LastCheckpointNotSet.into());
                }
                (false, None) => {
                    // If we don't have a last checkpoint, this means it's the first time we are importing checkpoints for the selected chain,
                    // so we need to make sure to set the last checkpoint to the newest one we import.
                    if let Some(ref mut last_checkpoint) = maybe_new_last_checkpoint {
                        if checkpoint.block_number > last_checkpoint.block_number {
                            maybe_new_last_checkpoint = Some(checkpoint.clone());
                        }
                    } else {
                        maybe_new_last_checkpoint = Some(checkpoint.clone());
                    }
                }
                _ => {
                    // In all other cases (including when we have neither attestations nor checkpoints),
                    // we allow the import of the checkpoint since it won't cause any conflict with the normal checkpointing flow
                }
            }

            Checkpoints::<T>::insert(chain_key, checkpoint.block_number, checkpoint.digest);
            CheckpointBuckets::<T>::insert(
                (
                    chain_key,
                    Self::compute_block_index_for(checkpoint.block_number),
                    checkpoint.block_number,
                ),
                (),
            );
            Self::deposit_event(Event::<T>::CheckpointReached(chain_key, checkpoint));
        }

        // If we updated the checkpoint storage, we also may need to update the last checkpoint.
        if let Some(checkpoint) = maybe_new_last_checkpoint {
            if checkpoint.block_number > initial_block_number || maybe_last_checkpoint.is_none() {
                // If the new checkpoint is higher than the initial checkpoint,
                // or if we didn't have a checkpoint before, we update the last checkpoint to the new one.
                LastCheckpoint::<T>::insert(chain_key, checkpoint);
            }
        }

        Ok(())
    }

    /// Clear attestations and checkpointing/removal queues plus [`LastDigest`] for `chain_key`.
    ///
    /// Used by [`forward_patch_checkpoints`] so repaired checkpoints are not contradicted by stale
    /// attestation rows (bounded by `FORWARD_PATCH_ATTESTATIONS_CLEAR_BATCH` ×
    /// `MAX_FORWARD_PATCH_ATTESTATIONS_CLEAR_LOOPS` entries per dispatch).
    pub(crate) fn purge_attestations_for_forward_patch(chain_key: ChainKey) -> DispatchResult {
        CheckpointingQueues::<T>::remove(chain_key);
        AttestationRemovalQueues::<T>::remove(chain_key);
        LastDigest::<T>::remove(chain_key);

        let mut maybe_cursor: Option<Vec<u8>> = None;
        for _ in 0..MAX_FORWARD_PATCH_ATTESTATIONS_CLEAR_LOOPS {
            let kill = Attestations::<T>::clear_prefix(
                chain_key,
                FORWARD_PATCH_ATTESTATIONS_CLEAR_BATCH,
                maybe_cursor.as_deref(),
            );
            maybe_cursor = kill.maybe_cursor;
            if maybe_cursor.is_none() {
                return Ok(());
            }
        }

        Err(Error::<T>::TooManyAttestationsForForwardPatchClear.into())
    }

    #[transactional]
    pub(crate) fn do_forward_patch_checkpoints(
        chain_key: ChainKey,
        wipe_suffix: bool,
        checkpoints: BoundedVec<AttestationCheckpoint, T::MaxCheckpointsImportedPerCall>,
    ) -> DispatchResult {
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );
        ensure!(
            CheckpointPruningStates::<T>::get(chain_key).is_none()
                && CheckpointClearingCursors::<T>::get(chain_key).is_none()
                && BucketClearingCursors::<T>::get(chain_key).is_none(),
            Error::<T>::CheckpointMaintenanceInProgress
        );
        ensure!(!checkpoints.is_empty(), Error::<T>::EmptyCheckpointPatch);

        Self::purge_attestations_for_forward_patch(chain_key)?;

        let mut merged = BTreeMap::new();
        for checkpoint in checkpoints.into_inner() {
            merged.insert(checkpoint.block_number, checkpoint.digest);
        }
        let (&batch_max, tip_digest) = merged
            .last_key_value()
            .ok_or(Error::<T>::EmptyCheckpointPatch)?;

        if wipe_suffix {
            let heights_above: Vec<u64> = Checkpoints::<T>::iter_prefix(chain_key)
                .filter_map(|(h, _)| (h > batch_max).then_some(h))
                .collect();
            ensure!(
                heights_above.len() <= MAX_CHECKPOINT_SUFFIX_WIPE_TOTAL,
                Error::<T>::CheckpointSuffixWipeTooLarge
            );
            for h in heights_above {
                Checkpoints::<T>::remove(chain_key, h);
                CheckpointBuckets::<T>::remove((chain_key, Self::compute_block_index_for(h), h));
            }
        }

        for (block_number, digest) in merged.iter() {
            let checkpoint = AttestationCheckpoint {
                block_number: *block_number,
                digest: *digest,
            };
            Checkpoints::<T>::insert(chain_key, *block_number, *digest);
            CheckpointBuckets::<T>::insert(
                (
                    chain_key,
                    Self::compute_block_index_for(*block_number),
                    *block_number,
                ),
                (),
            );
            Self::deposit_event(Event::<T>::CheckpointReached(chain_key, checkpoint));
        }

        let batch_tip = AttestationCheckpoint {
            block_number: batch_max,
            digest: *tip_digest,
        };
        let new_last = match LastCheckpoint::<T>::get(chain_key) {
            Some(lc) if !wipe_suffix && lc.block_number > batch_tip.block_number => lc,
            _ => batch_tip,
        };
        LastCheckpoint::<T>::insert(chain_key, new_last.clone());

        Self::deposit_event(Event::<T>::ForwardCheckpointPatchApplied {
            chain_key,
            wiped_suffix: wipe_suffix,
            tip_block_number: new_last.block_number,
        });

        Ok(())
    }

    #[inline]
    pub fn compute_block_index_for(block_number: u64) -> u64 {
        block_number - (block_number % CHECKPOINT_BUCKET_SIZE)
    }
}
// helper functions for checking inherent data
impl<T: Config> Pallet<T> {
    pub(crate) fn check_duplicate(attestation: &SignedAttestation<T::Hash, T::AccountId>) -> bool {
        let digest = attestation.digest();
        if Attestations::<T>::get(attestation.attestation.chain_key, digest).is_some() {
            log::debug!("Attestation with digest: {digest:?} is duplicate");
            return true;
        }

        // Get last checkpoint
        if let Some(checkpoint) = LastCheckpoint::<T>::get(attestation.attestation.chain_key) {
            if attestation.header_number() <= checkpoint.block_number {
                log::debug!(
                    "Attestation with block number: {:?} is duplicate",
                    attestation.header_number()
                );
                return true;
            }
        }

        false
    }

    pub(crate) fn extract_agg_signature(signature: &[u8]) -> Result<Signature, Error<T>> {
        Signature::from_bytes(signature).map_err(|_| {
            log::error!("Failed to aggregate BLS signature");
            Error::<T>::InvalidBlsSignature
        })
    }

    pub(crate) fn gather_attestor_public_keys(
        chain_key: ChainKey,
        attestors: &[T::AccountId],
    ) -> Result<Vec<PublicKey>, Error<T>> {
        attestors
            .iter()
            .map(|attestor_id| {
                let attestor_rec = Attestors::<T>::get(chain_key, attestor_id);
                let retired = RetiredAttestorBlsKeys::<T>::get(chain_key, attestor_id);
                let has_retired = retired.is_some();
                let maybe_key = attestor_rec
                    .as_ref()
                    .and_then(|a| a.bls_public_key)
                    .or_else(|| retired.map(|e| e.bls_public_key));
                let key = match maybe_key {
                    Some(k) => k,
                    None => {
                        return Err(if attestor_rec.is_some() || has_retired {
                            log::error!("No BLS key for attestor {attestor_id:?}");
                            Error::<T>::AttestorWithInvalidPublicKey
                        } else {
                            log::error!("Attestor {attestor_id:?} not found");
                            Error::<T>::InvalidAttestorFound
                        });
                    }
                };
                PublicKey::from_bytes(&key[..]).map_err(|_| {
                    log::error!("Invalid BLS key for attestor {attestor_id:?}");
                    Error::<T>::AttestorWithInvalidPublicKey
                })
            })
            .collect()
    }

    pub(crate) fn verify_agg_signature(
        agg_signature: &Signature,
        message: &[u8],
        agg_public_key: PublicKey,
    ) -> Result<(), Error<T>> {
        if !bls_signatures::verify_agg_message(agg_signature, message, agg_public_key) {
            log::error!("Aggregated signature is invalid");
            return Err(Error::<T>::InvalidBlsSignature);
        }
        Ok(())
    }

    #[transactional]
    fn remove_attestations(
        chain_key: ChainKey,
        mut attestation_removal_queue: VecDeque<Digest>,
    ) -> DispatchResult {
        let retention_duration = AttestationRetentionDuration::<T>::get(chain_key);
        while attestation_removal_queue.len() > retention_duration as usize {
            if let Some(to_remove) = attestation_removal_queue.pop_front() {
                Attestations::<T>::take(chain_key, to_remove);
            } else {
                return Err(Error::<T>::CheckpointCreationError.into());
            }
        }
        AttestationRemovalQueues::<T>::insert(chain_key, attestation_removal_queue);
        Ok(())
    }
}

/// TRAIT IMPLS ///
impl<T: Config> OnRandomnessUpdate for Pallet<T> {
    fn on_new_epoch_randomness(epoch: u64, randomness: Randomness) {
        // Start new election
        debug!("on_new_epoch_randomness: epoch: {epoch}, randomness: {randomness:?}",);

        match Self::do_start_election(epoch, randomness) {
            Ok(_) => (),
            Err(e) => {
                log::error!("Error starting election: {e:?}");
            }
        }

        // We also apply attestation interval updates, if any, at epoch boundaries.
        // Change attestation intervals and emit events
        Self::apply_interval_updates();
    }
}
