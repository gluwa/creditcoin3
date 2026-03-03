use frame_support::{
    pallet_prelude::*,
    traits::{Currency, DefensiveSaturating},
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
use supported_chains_primitives::{
    chain_removal_listener::ChainRemovalListener, provider::SupportedChainsProvider,
};

use crate::{
    asset::existential_deposit,
    ledger::{AttestorLedger, UnlockChunk},
};

use super::pallet::*;

// One tenth of a CTC in micro units
pub const ONE_TENTH_CTC: u64 = 100_000_000_000_000_000;

/// PALLET CALL IMPLS ///
impl<T: Config> Pallet<T> {
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

        ensure!(
            !Self::attestor_is_registered(chain_key, &attestor_id),
            Error::<T>::AlreadyAttestor
        );

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

        if !value.is_zero() {
            // Decrease the active amount
            ledger.active -= value;

            // Avoid there being a dust balance left in the staking system.
            if ledger.active < existential_deposit::<T>() {
                value += ledger.active;
                ledger.active = Zero::zero();
            }

            // Note: in case there is no current era it is fine to bond one era more.
            let era = Self::current_era().defensive_saturating_add(T::BondingDuration::get());
            if let Some(chunk) = ledger.unlocking.last_mut().filter(|chunk| chunk.era == era) {
                // To keep the chunk count down, we only keep one chunk per era. Since
                // `unlocking` is a FiFo queue, if a chunk exists for `era` we know that it will
                // be the last one.
                chunk.value = chunk.value.defensive_saturating_add(value)
            } else {
                ledger
                    .unlocking
                    .try_push(UnlockChunk { value, era })
                    .map_err(|_| Error::<T>::NoMoreChunks)?;
            };

            // Update the ledger
            ledger.update()?;

            Self::deposit_event(Event::<T>::Unbonded {
                stash,
                amount: value,
            });
        }

        // Remove the attestor
        Attestors::<T>::remove(chain_key, &attestor_id);

        Self::deposit_event(Event::<T>::AttestorUnregistered(chain_key, attestor_id));

        Ok(())
    }

    pub(crate) fn do_bootstrap_chain(
        attestation: SignedAttestation<T::Hash, T::AccountId>,
    ) -> DispatchResult {
        let chain_key = attestation.chain_key();
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );

        let previous_digest = Self::last_digest(chain_key);

        // Store the attestation
        let digest = attestation.digest();
        let header_number = attestation.header_number();
        Attestations::<T>::insert(chain_key, digest, &attestation);

        // Update last digest
        LastDigest::<T>::set(chain_key, Some((header_number, digest)));

        Self::deposit_event(Event::<T>::BlockAttested(chain_key, header_number, digest));

        match previous_digest {
            None => {
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
            }
            Some(_prev_digest) => {
                // Add to checkpointing queue
                let mut queue = CheckpointingQueues::<T>::get(chain_key);
                queue.push_back(digest);

                // Make checkpoint if necessary.
                // The extrinsic didn't fail even if checkpointing failed. We want
                // to keep the new attestation rather than removing it from storage
                // via extrinsic rollback in the case of checkpointing failure.
                if let Err(e) = Self::try_make_checkpoint(&mut queue, chain_key, header_number) {
                    log::error!("Error: {e:?}");
                }
                CheckpointingQueues::<T>::insert(chain_key, queue);
            }
        }

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
        LastDigest::<T>::set(chain_key, Some((header_number, digest)));

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

    pub(crate) fn do_chill_attestor(chain_key: ChainKey, attestor_id: T::AccountId) {
        Attestors::<T>::mutate(chain_key, &attestor_id, |maybe_attestor| {
            if let Some(attestor) = maybe_attestor {
                attestor.status = AttestorStatus::Idle;
            }
        });

        Self::deposit_event(Event::<T>::AttestorChilled(chain_key, attestor_id));
    }

    pub(super) fn do_withdraw_unbonded(stash: &T::AccountId) -> DispatchResult {
        let mut ledger = Self::ledger(stash).ok_or(Error::<T>::NotStash)?;

        let (stash, old_total) = (ledger.stash.clone(), ledger.total_staked);

        let current_era = Self::current_era();
        if current_era > 0 {
            ledger = ledger.consolidate_unlocked(current_era)
        }
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

    fn chill_all_attestors_for_chain(chain_key: ChainKey) {
        let attestor_ids = Attestors::<T>::iter_key_prefix(chain_key);
        for attestor_id in attestor_ids {
            Self::do_chill_attestor(chain_key, attestor_id);
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
        let count = Attestors::<T>::iter_prefix_values(chain_key)
            .collect::<Vec<_>>()
            .len() as u32;
        count < MaxAttestors::<T>::get(chain_key)
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

        ensure!(
            attestation
                .attestors
                .iter()
                .all(|attestor| active_attestors.contains(attestor)),
            Error::<T>::AttestorNotActive
        );

        // Ensure no duplicate attestors
        let unique_attestors: BTreeSet<&T::AccountId> = attestation.attestors.iter().collect();
        ensure!(
            unique_attestors.len() == attestation.attestors.len(),
            Error::<T>::DuplicateAttestor
        );

        // Threshold validation
        let target_sample_size = Self::target_sample_size(chain_key);
        let threshold = calculate_threshold(target_sample_size);
        ensure!(
            attestation.attestors.len() as u32 >= threshold,
            Error::<T>::MajorityNotReached
        );

        // Signature verification
        let agg_signature = Self::extract_agg_signature(&attestation.signature)?;
        let attestor_public_keys =
            Self::gather_attestor_public_keys(attestation.chain_key(), &attestation.attestors)?;
        let aggregated_public_key =
            aggregate_public_keys(&attestor_public_keys[..]).map_err(|_| {
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
        // rolled back.
        let mut checkpointing_rollback: Vec<Digest> = Vec::new();

        let new_checkpoint = loop {
            let attestation_digest: Digest = match queue.pop_front() {
                Some(digest) => digest,
                None => {
                    for digest in checkpointing_rollback {
                        queue.push_front(digest);
                    }
                    return Err(Error::<T>::CheckpointingQueueDrained.into());
                }
            };
            checkpointing_rollback.push(attestation_digest);

            let attestation = match Attestations::<T>::get(chain_key, attestation_digest) {
                Some(attestation) => attestation,
                None => {
                    for digest in checkpointing_rollback {
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
                    // that matches the target block we want to condense to
                    let mut maybe_digest = None;
                    for proof in attestation.continuity_proof.iter() {
                        if proof.block_number == target_block {
                            maybe_digest = Some(proof.digest);
                            break;
                        }
                    }

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
                            for digest in checkpointing_rollback {
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
        let proof_len = attestation.continuity_proof.get_blocks_ref().len();
        if proof_len < 2 * checkpoint_width as usize {
            return Ok(false);
        }

        // Build map of block_number -> digest from continuity proof blocks.
        // Include the attestation block itself so boundaries at header_number are not missed.
        let mut block_digests: BTreeMap<u64, Digest> = attestation
            .continuity_proof
            .get_blocks_ref()
            .iter()
            .map(|b| (b.block_number, b.digest))
            .collect();
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
                // Add all blocks from its continuity proof
                for block in queued_att.continuity_proof.get_blocks_ref() {
                    block_digests
                        .entry(block.block_number)
                        .or_insert(block.digest);
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
        // Attesting should start after all checkpoints are imported.
        // Otherwise we introduce poorly defined behavior.
        ensure!(
            LastDigest::<T>::get(chain_key).is_none(),
            Error::<T>::AttestationFoundWhileImporting
        );

        let stored_last_checkpoint = LastCheckpoint::<T>::get(chain_key);
        let mut last_checkpoint = stored_last_checkpoint.clone().unwrap_or_default();
        let initial_block_number = last_checkpoint.block_number;

        for checkpoint in checkpoints {
            if Checkpoints::<T>::contains_key(chain_key, checkpoint.block_number) {
                continue;
            }

            if checkpoint.block_number >= last_checkpoint.block_number {
                last_checkpoint = checkpoint.clone();
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

        if last_checkpoint.block_number > initial_block_number || stored_last_checkpoint.is_none() {
            LastCheckpoint::<T>::insert(chain_key, last_checkpoint);
        }
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
            log::error!("Attestation with digest: {digest:?} is duplicate");
            return true;
        }

        // Get last checkpoint
        if let Some(checkpoint) = LastCheckpoint::<T>::get(attestation.attestation.chain_key) {
            if attestation.header_number() <= checkpoint.block_number {
                log::error!(
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
            .map(|attestor| {
                let attestor = Attestors::<T>::get(chain_key, attestor).ok_or_else(|| {
                    log::error!("Attestor {attestor:?} not found");
                    Error::<T>::InvalidAttestorFound
                })?;
                match attestor.bls_public_key {
                    Some(key) => PublicKey::from_bytes(&key[..]).map_err(|_| {
                        log::error!("Invalid BLS key for attestor {attestor:?}");
                        Error::<T>::AttestorWithInvalidPublicKey
                    }),
                    None => {
                        log::error!("No BLS key for attestor {attestor:?}");
                        Err(Error::<T>::AttestorWithInvalidPublicKey)
                    }
                }
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

impl<T: Config> ChainRemovalListener for Pallet<T> {
    fn on_supported_chain_removed(chain_key: ChainKey, remove_checkpoints: bool) {
        Self::chill_all_attestors_for_chain(chain_key);

        ActiveAttestors::<T>::remove(chain_key);

        // Can dispense with result, since limit is equal to maximum storage size
        _ = Invulnerables::<T>::clear_prefix(
            chain_key,
            MaxInvulnerables::<T>::get(chain_key),
            None,
        );

        MaxAttestors::<T>::remove(chain_key);

        MaxInvulnerables::<T>::remove(chain_key);

        // Clearing attestations
        let max_attestations_to_remove = AttestationCheckpointInterval::<T>::get(chain_key) * 2 + 1;
        // Can dispense with result, since limit is equal to maximum storage size
        _ = Attestations::<T>::clear_prefix(chain_key, max_attestations_to_remove, None);

        CheckpointingQueues::<T>::remove(chain_key);
        LastCheckpoint::<T>::remove(chain_key);
        LastDigest::<T>::remove(chain_key);
        PendingTargetSampleSize::<T>::remove(chain_key);
        TargetSampleSize::<T>::remove(chain_key);
        ChainAttestationInterval::<T>::remove(chain_key);
        PendingAttestationInterval::<T>::remove(chain_key);
        AttestationCheckpointInterval::<T>::remove(chain_key);
        MaxCatchup::<T>::remove(chain_key);
        PendingMaxCatchup::<T>::remove(chain_key);

        if remove_checkpoints {
            // Starting the process of clearing checkpoints. There may be a very large number of checkpoints
            // in storage, and we aren't in a huge hurry to clear them out. So we clear a moderate number per
            // block.
            let maybe_cursor = Checkpoints::<T>::clear_prefix(
                chain_key,
                u32::from(MAX_CHECKPOINTS_CLEARED_PER_BLOCK),
                None,
            )
            .maybe_cursor;

            if maybe_cursor.is_some() {
                // more checkpoints left to be removed
                // Attestation pallet will check this storage to trigger further checkpoint removals in future blocks
                // and CheckpointsCleared event will be dispatched inside on_initialize()
                CheckpointClearingCursors::<T>::set(chain_key, maybe_cursor);
            } else {
                // all checkpoints were removed in the call above, trigger the event here
                // b/c on_initialize() won't do that
                Self::deposit_event(Event::<T>::CheckpointsCleared(chain_key));
            }
        }

        Self::deposit_event(Event::<T>::ClearedStorageForRemovedChain(chain_key));
    }
}
