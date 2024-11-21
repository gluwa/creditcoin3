use frame_support::{
    pallet_prelude::*,
    traits::{Currency, DefensiveSaturating, OnUnbalanced},
    transactional,
};
use log::{debug, info};
use sp_runtime::{
    traits::{CheckedAdd, CheckedSub, SaturatedConversion, Saturating, Zero},
    ArithmeticError,
};
use sp_staking::StakingInterface;
use sp_std::collections::btree_map::BTreeMap;
use sp_std::collections::vec_deque::VecDeque;
use sp_std::vec::Vec;

use attestor_primitives::{
    AttestationCheckpoint, Attestor, AttestorStatus, BlsPublicKey, BlsSignature, ChainKey, Digest,
    InherentError, SignedAttestation,
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
            stash_balance >= Self::min_bond_requirement(),
            Error::<T>::InsufficientBalance
        );

        // Create a new ledger for the attestor
        // With minimum bond requirement
        let ledger: AttestorLedger<T> =
            AttestorLedger::new(stash.clone(), Self::min_bond_requirement());

        // Default to stash as payee
        // If bond fails, it means it's already bonded and there is already an attestor(s) registerd by this stash
        // In this case, we just bond extra to the stash
        if ledger.bond(RewardDestination::Stash).is_err() {
            Self::bond_extra(&stash)?;
        } else {
            // Would fail if account has no provider.
            frame_system::Pallet::<T>::inc_consumers(&stash)?;

            // Emit event
            Self::deposit_event(Event::<T>::Bonded {
                stash,
                amount: Self::min_bond_requirement(),
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
        let bond = Self::min_bond_requirement();

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
        LastDigest::<T>::set(chain_key, Some(digest));

        Self::deposit_event(Event::<T>::BlockAttested(
            chain_key,
            attestation.clone(),
            digest,
        ));

        match previous_digest {
            None => {
                // Very first attestation should have a corresponding checkpoint
                // even though it doesn't condense any prior attestations.
                let checkpoint = AttestationCheckpoint {
                    block_number: header_number,
                    digest,
                };

                Self::deposit_event(Event::<T>::CheckpointReached(chain_key, checkpoint.clone()));

                Checkpoints::<T>::insert(chain_key, checkpoint.digest, checkpoint);
            }
            Some(_prev_digest) => {
                // Add to checkpointing queue
                let mut queue = CheckpointingQueues::<T>::get(chain_key);
                queue.push_back(digest);

                // Make checkpoint if necessary.
                // The extrinsic didn't fail even if checkpointing failed. We want
                // to keep the new attestation rather than removing it from storage
                // via extrinsic rollback in the case of checkpointing failure.
                if let Err(e) = Self::try_make_checkpoint(&mut queue, chain_key) {
                    log::error!("Error: {:?}", e);
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
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );

        ensure!(
            !Attestations::<T>::contains_key(chain_key, attestation.digest()),
            Error::<T>::AttestationExists
        );

        ensure!(
            Self::validate_attestation(&attestation).is_ok(),
            Error::<T>::InvalidAttestation
        );

        let previous_digest = Self::last_digest(chain_key);
        ensure!(
            previous_digest == attestation.attestation.prev_digest,
            Error::<T>::InvalidAttestation
        );

        if let Some(previous_digest) = previous_digest {
            let previous_attestation = Attestations::<T>::get(chain_key, previous_digest)
                .ok_or(Error::<T>::NoPreviousDigest)?;

            let interval = ChainAttestationInterval::<T>::get(chain_key);
            let prev_block_number = previous_attestation.attestation.header_number;

            debug!(
                "Checking if block number is at the interval, expected: {:?}, got: {:?}",
                prev_block_number + interval,
                attestation.attestation.header_number
            );

            if attestation.attestation.header_number != prev_block_number + interval {
                debug!(
                    "Block number is not at the interval, expected: {:?}, got: {:?}",
                    prev_block_number + interval,
                    attestation.attestation.header_number
                );
                return Err(Error::<T>::InvalidAttestation.into());
            }
        }

        // Store the attestation
        let digest = attestation.digest();
        let header_number = attestation.header_number();
        Attestations::<T>::insert(chain_key, digest, &attestation);

        // Update last digest
        LastDigest::<T>::set(chain_key, Some(digest));

        // Pay out attestation rewards
        Self::payout_attestors(chain_key, &attestation.attestors)?;

        // Emit event
        Self::deposit_event(Event::<T>::BlockAttested(chain_key, attestation, digest));

        match previous_digest {
            None => {
                // Very first attestation should have a corresponding checkpoint
                // even though it doesn't condense any prior attestations.
                let checkpoint = AttestationCheckpoint {
                    block_number: header_number,
                    digest,
                };

                Self::deposit_event(Event::<T>::CheckpointReached(chain_key, checkpoint.clone()));

                Checkpoints::<T>::insert(chain_key, checkpoint.digest, checkpoint);
            }
            Some(_prev_digest) => {
                // Add to checkpointing queue
                let mut queue = CheckpointingQueues::<T>::get(chain_key);
                queue.push_back(digest);

                // Make checkpoint if necessary.
                // The extrinsic didn't fail even if checkpointing failed. We want
                // to keep the new attestation rather than removing it from storage
                // via extrinsic rollback in the case of checkpointing failure.
                if let Err(e) = Self::try_make_checkpoint(&mut queue, chain_key) {
                    log::error!("Error: {:?}", e);
                }
                CheckpointingQueues::<T>::insert(chain_key, queue);
            }
        }

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

        attestor.status = AttestorStatus::Active;
        attestor.bls_public_key = Some(bls_public_key);
        Attestors::<T>::insert(chain_key, &attestor_id, attestor);

        Self::deposit_event(Event::<T>::AttestorActivated(chain_key, attestor_id));

        Ok(())
    }

    pub(crate) fn do_chill_attestor(
        chain_key: ChainKey,
        attestor_id: T::AccountId,
        mut attestor: Attestor<T::AccountId>,
    ) {
        attestor.status = AttestorStatus::Idle;
        Attestors::<T>::insert(chain_key, &attestor_id, attestor);

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

    /// Claim the rewards for the given stash
    /// The rewards are transferred to the reward destination
    /// If the reward destination is not set, the rewards are not claimed
    pub(super) fn do_claim_rewards(stash: T::AccountId) -> DispatchResult {
        let amount = AccumulatedRewards::<T>::take(&stash).ok_or(Error::<T>::NoRewards)?;
        if amount.is_zero() {
            return Ok(());
        }

        let imbalance = if let Some(payee) = Payee::<T>::get(&stash) {
            match payee {
                // No reward destination, do nothing
                RewardDestination::None => return Ok(()),
                RewardDestination::Account(a) => T::Currency::deposit_creating(&a, amount),
                RewardDestination::Stash => T::Currency::deposit_into_existing(&stash, amount)?,
            }
        } else {
            // Transfer the amount to the reward destination
            T::Currency::deposit_into_existing(&stash, amount)?
        };

        // Make sure we try to drop any imbalance that may have occurred
        T::Reward::on_unbalanced(imbalance);

        Self::deposit_event(Event::<T>::RewardClaimed { stash, amount });

        Ok(())
    }
}

/// NON-CALL FUNCTIONS ///
impl<T: Config> Pallet<T> {
    pub(super) fn bond_extra(stash: &T::AccountId) -> DispatchResult {
        let bond = Self::min_bond_requirement();

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

    /// Payout the rewards to the attestors
    /// This actually saves all the rewards in the `AccumulatedRewards` storage item
    /// A stash can manually withdraw the rewards by calling `claim_rewards`
    pub(super) fn payout_attestors(chain_key: u64, attestors: &[T::AccountId]) -> DispatchResult {
        // Retrieve the reward amount for the given chain key or return an error if not found
        let reward = ChainReward::<T>::get(chain_key).ok_or(Error::<T>::ChainRewardNotFound)?;

        // Create a map to store total rewards per stash
        let mut total_per_stash: BTreeMap<T::AccountId, BalanceOf<T>> = BTreeMap::new();

        // Accumulate the rewards for each attestor
        for attestor in attestors {
            let stash = Attestors::<T>::get(chain_key, attestor)
                .ok_or(Error::<T>::AddressNotAttestor)?
                .stash;

            // Increment the total reward for each stash
            total_per_stash
                .entry(stash)
                .and_modify(|total| *total += reward)
                .or_insert(reward);
        }

        // Update the accumulated rewards for each stash in storage
        for (stash, total) in total_per_stash {
            // Deposit the reward event
            Self::deposit_event(Event::<T>::RewardPaid {
                chain_key,
                stash: stash.clone(),
                amount: total,
            });

            // Update the accumulated rewards for the stash
            AccumulatedRewards::<T>::mutate(stash, |accumulated| {
                if let Some(ref mut rewards) = accumulated {
                    *rewards += total;
                } else {
                    *accumulated = Some(total);
                }
            });
        }

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
    pub fn do_start_election(epoch: u64, _randomness: Randomness) -> DispatchResult {
        let supported_chains =
            T::SupportedChains::supported_chains().ok_or(Error::<T>::NoSupportedChains)?;

        for chain_key in supported_chains {
            let prefix = Attestors::<T>::iter_prefix(chain_key);

            let attestors = prefix
                .filter_map(|(account, attestor)| {
                    if attestor.status == AttestorStatus::Active {
                        Some(account)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>();

            if attestors.is_empty() {
                info!("No active attestors for chain {}", chain_key);
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

    fn apply_interval_updates() {
        PendingAttestationInterval::<T>::iter().for_each(
            |(chain_key, new_attestation_interval)| {
                ChainAttestationInterval::<T>::set(chain_key, new_attestation_interval);

                Self::deposit_event(Event::<T>::AttestationIntervalChanged(
                    chain_key,
                    new_attestation_interval,
                ));
            },
        );

        // Clear PendingAttestationInterval
        let num_supported_chains = T::SupportedChains::supported_chains()
            .unwrap_or_default()
            .len();
        let _ = PendingAttestationInterval::<T>::clear(num_supported_chains as u32, None);
    }

    fn chill_all_attestors_for_chain(chain_key: ChainKey) {
        let attestors = Attestors::<T>::iter_prefix(chain_key);
        for (attestor_id, attestor) in attestors {
            Self::do_chill_attestor(chain_key, attestor_id, attestor);
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
    }

    pub fn contains_digest(chain_key: ChainKey, digest: Digest) -> bool {
        Attestations::<T>::contains_key(chain_key, digest)
    }

    pub fn attestor_bls_pubkey(
        chain_key: ChainKey,
        address: &T::AccountId,
    ) -> Option<BlsPublicKey> {
        let pk = Attestors::<T>::get(chain_key, address).map(|attestor| attestor.bls_public_key);
        match pk {
            Some(pk) => pk,
            None => None,
        }
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

    pub(crate) fn validate_attestation(
        attestation: &SignedAttestation<T::Hash, T::AccountId>,
    ) -> Result<(), InherentError> {
        Self::check_duplicate(attestation)?;
        let agg_signature = Self::extract_agg_signature(&attestation.signature)?;
        let attestor_public_keys =
            Self::gather_attestor_public_keys(attestation.chain_key(), &attestation.attestors)?;
        let aggregated_public_key =
            aggregate_public_keys(&attestor_public_keys[..]).map_err(|_| {
                log::error!("Failed to aggregate public keys");
                InherentError::NotValid
            })?;

        let message = &attestation.attestation.serialize()[..];

        Self::verify_agg_signature(&agg_signature, message, aggregated_public_key)?;

        log::info!("Attestation signature is valid");

        Ok(())
    }

    // When current checkpoint interval is completed by the commitment of its final attestation,
    // then the prior checkpoint interval is considered "stabilized". We condense all the
    // attestations for that prior interval into a single checkpoint.
    #[transactional]
    pub(crate) fn try_make_checkpoint(
        queue: &mut VecDeque<Digest>,
        chain_key: ChainKey,
    ) -> DispatchResult {
        let num_to_condense = Self::attestation_checkpoint_interval(chain_key);
        // Only move forward if two full checkpoints of attestations are committed.
        if queue.len() < (num_to_condense * 2) as usize {
            return Ok(());
        }

        // Because checkpointing queue storage is written to after this function
        // returns, it isn't covered by the #[transactional] macro and must be manually
        // rolled back.
        let mut checkpointing_rollback: Vec<Digest> = Vec::new();
        for i in 0..num_to_condense {
            let to_be_removed: Digest = match queue.pop_front() {
                Some(digest) => digest,
                None => {
                    for digest in checkpointing_rollback {
                        queue.push_front(digest);
                    }
                    return Err(Error::<T>::CheckpointCreationError.into());
                }
            };
            checkpointing_rollback.push(to_be_removed);

            // Until then, removing attestations from storage breaks proving.
            let removed = match Attestations::<T>::take(chain_key, to_be_removed) {
                Some(attestation) => attestation,
                None => {
                    for digest in checkpointing_rollback {
                        queue.push_front(digest);
                    }
                    return Err(Error::<T>::CheckpointCreationError.into());
                }
            };

            if i == num_to_condense - 1 {
                let checkpoint = AttestationCheckpoint {
                    block_number: removed.header_number(),
                    digest: removed.digest(),
                };

                Self::deposit_event(Event::<T>::CheckpointReached(chain_key, checkpoint.clone()));

                Checkpoints::<T>::insert(chain_key, checkpoint.digest, checkpoint);
            }
        }

        Ok(())
    }
}
// helper functions for checking inherent data
impl<T: Config> Pallet<T> {
    pub(crate) fn check_duplicate(
        attestation: &SignedAttestation<T::Hash, T::AccountId>,
    ) -> Result<(), InherentError> {
        if let Some(digest) = LastDigest::<T>::get(attestation.attestation.chain_key) {
            if digest == attestation.attestation.digest() {
                log::error!("Attestation with digest: {:?} is duplicate", digest);
                return Err(InherentError::Duplicate(digest));
            }
        }
        Ok(())
    }

    pub(crate) fn extract_agg_signature(signature: &[u8]) -> Result<Signature, InherentError> {
        Signature::from_bytes(signature).map_err(|_| {
            log::error!("Failed to aggregate BLS signature");
            InherentError::NotValid
        })
    }

    pub(crate) fn gather_attestor_public_keys(
        chain_key: ChainKey,
        attestors: &[T::AccountId],
    ) -> Result<Vec<PublicKey>, InherentError> {
        attestors
            .iter()
            .map(|attestor| {
                let active_attestors = ActiveAttestors::<T>::get(chain_key);
                let contains = active_attestors.contains(attestor);

                if contains {
                    let attestor = Attestors::<T>::get(chain_key, attestor).ok_or_else(|| {
                        log::error!("Attestor {:?} not found", attestor);
                        InherentError::InvalidAttestorFound
                    })?;
                    match attestor.bls_public_key {
                        Some(key) => PublicKey::from_bytes(&key[..]).map_err(|_| {
                            log::error!("Invalid BLS key for attestor {:?}", attestor);
                            InherentError::AttestorWithInvalidPublicKey
                        }),
                        None => {
                            log::error!("No BLS key for attestor {:?}", attestor);
                            Err(InherentError::AttestorWithInvalidPublicKey)
                        }
                    }
                } else {
                    log::error!("Attestor {:?} is not active", attestor);
                    Err(InherentError::AttestorNotActive)
                }
            })
            .collect()
    }

    pub(crate) fn verify_agg_signature(
        agg_signature: &Signature,
        message: &[u8],
        agg_public_key: PublicKey,
    ) -> Result<(), InherentError> {
        if !bls_signatures::verify_agg_message(agg_signature, message, agg_public_key) {
            log::error!("Aggregated signature is invalid");
            return Err(InherentError::NotValid);
        }
        Ok(())
    }
}

/// TRAIT IMPLS ///

impl<T: Config> OnRandomnessUpdate for Pallet<T> {
    fn on_new_epoch_randomness(epoch: u64, randomness: Randomness) {
        // Start new election
        info!(
            "on_new_epoch_randomness: epoch: {}, randomness: {:?}",
            epoch, randomness
        );

        match Self::do_start_election(epoch, randomness) {
            Ok(_) => (),
            Err(e) => {
                log::error!("Error starting election: {:?}", e);
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
        LastDigest::<T>::remove(chain_key);
        CommitteeSetSize::<T>::remove(chain_key);
        ChainAttestationInterval::<T>::remove(chain_key);
        PendingAttestationInterval::<T>::remove(chain_key);
        AttestationCheckpointInterval::<T>::remove(chain_key);
        ChainReward::<T>::remove(chain_key);

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
            // Attestation pallet will check this storage to trigger further checkpoint removals in future blocks
            CheckpointClearingCursors::<T>::set(chain_key, maybe_cursor);
        }

        Self::deposit_event(Event::<T>::ClearedStorageForRemovedChain(chain_key));
    }
}
