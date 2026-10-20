//! Controller-authorized networking identities; independent of the existing Attestor encoding.

use super::*;
use attestor_primitives::{
    peer_key_proof_message, AttestorStatus, ChainKey, PeerPublicKey, PeerRegistry,
};
use frame_support::ensure;
use parity_scale_codec::Encode;
use supported_chains_primitives::provider::SupportedChainsProvider;

impl<T: Config> Pallet<T> {
    pub(crate) fn do_set_attestor_peer_key(
        chain_key: ChainKey,
        controller: T::AccountId,
        public_key: PeerPublicKey,
        proof: [u8; 64],
        expected_previous_key: Option<PeerPublicKey>,
    ) -> frame_support::dispatch::DispatchResult {
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );
        ensure!(
            Attestors::<T>::contains_key(chain_key, &controller),
            Error::<T>::AddressNotAttestor
        );

        let genesis = frame_system::Pallet::<T>::block_hash(
            frame_system::pallet_prelude::BlockNumberFor::<T>::default(),
        );
        let message = peer_key_proof_message(
            genesis.as_ref(),
            chain_key,
            &controller.encode(),
            &public_key,
        );
        ensure!(
            sp_io::crypto::ed25519_verify(
                &sp_core::ed25519::Signature::from_raw(proof),
                &message,
                &sp_core::ed25519::Public::from_raw(public_key),
            ),
            Error::<T>::InvalidPeerKeyProof
        );
        ensure!(
            PeerKeyOwner::<T>::get(chain_key, public_key).is_none_or(|owner| owner == controller),
            Error::<T>::PeerKeyAlreadyRegistered
        );
        let current = AttestorPeerKeys::<T>::get(chain_key, &controller);
        if current == Some(public_key) {
            return Ok(());
        }
        ensure!(
            current == expected_previous_key,
            Error::<T>::PeerKeyRegistrationChanged
        );

        // Validate all fallible conditions before changing either side of the index.
        Self::clear_attestor_peer_key(chain_key, &controller);
        AttestorPeerKeys::<T>::insert(chain_key, &controller, public_key);
        PeerKeyOwner::<T>::insert(chain_key, public_key, &controller);
        Self::deposit_event(Event::<T>::AttestorPeerKeyUpdated(
            chain_key, controller, public_key,
        ));
        Ok(())
    }

    pub(crate) fn clear_attestor_peer_key(chain_key: ChainKey, controller: &T::AccountId) {
        if let Some(key) = AttestorPeerKeys::<T>::take(chain_key, controller) {
            if PeerKeyOwner::<T>::get(chain_key, key).as_ref() == Some(controller) {
                PeerKeyOwner::<T>::remove(chain_key, key);
            }
        }
    }

    pub(crate) fn do_set_peer_registration_required(
        chain_key: ChainKey,
        required: bool,
    ) -> frame_support::dispatch::DispatchResult {
        ensure!(
            T::SupportedChains::is_chain_supported(chain_key),
            Error::<T>::ChainNotSupported
        );
        if required {
            // Includes Waiting and Leaving so the next election cannot introduce a missing
            // key. Idle registrations may backfill before their next activation.
            for (controller, attestor) in Attestors::<T>::iter_prefix(chain_key) {
                if attestor.status != attestor_primitives::AttestorStatus::Idle {
                    ensure!(
                        AttestorPeerKeys::<T>::contains_key(chain_key, controller),
                        Error::<T>::PeerKeyNotRegistered
                    );
                }
            }
            for controller in ActiveAttestors::<T>::get(chain_key) {
                ensure!(
                    AttestorPeerKeys::<T>::contains_key(chain_key, controller),
                    Error::<T>::PeerKeyNotRegistered
                );
            }
        }
        PeerRegistrationRequired::<T>::insert(chain_key, required);
        Self::deposit_event(Event::<T>::PeerRegistrationRequiredChanged(
            chain_key, required,
        ));
        Ok(())
    }

    pub fn attestor_peer_registry(chain_key: ChainKey) -> PeerRegistry<T::AccountId> {
        let active = ActiveAttestors::<T>::get(chain_key);
        let policy = ChainElectionPolicy::<T>::get(chain_key);
        let waiting_attestors = Attestors::<T>::iter_prefix(chain_key)
            .filter(|(id, attestor)| {
                attestor.status == AttestorStatus::Waiting
                    && !active.contains(id)
                    && match policy {
                        AttestorElectionPolicy::OpenToAny => true,
                        AttestorElectionPolicy::AuthorizedOnly => {
                            AuthorizedAttestors::<T>::contains_key(chain_key, id)
                        }
                        AttestorElectionPolicy::DeniedToAll => false,
                    }
            })
            .map(|(id, _)| {
                let key = AttestorPeerKeys::<T>::get(chain_key, &id);
                (id, key)
            })
            .collect();
        PeerRegistry {
            registration_required: PeerRegistrationRequired::<T>::get(chain_key),
            attestors: active
                .into_iter()
                .map(|id| {
                    let key = AttestorPeerKeys::<T>::get(chain_key, &id);
                    (id, key)
                })
                .collect(),
            waiting_attestors,
        }
    }

    #[cfg(feature = "try-runtime")]
    pub(crate) fn check_peer_key_invariants() -> Result<(), sp_runtime::TryRuntimeError> {
        for (chain, controller, key) in AttestorPeerKeys::<T>::iter() {
            ensure!(
                Attestors::<T>::contains_key(chain, &controller),
                "peer key has no registered controller"
            );
            ensure!(
                PeerKeyOwner::<T>::get(chain, key) == Some(controller),
                "peer key owner index differs"
            );
        }
        for (chain, key, controller) in PeerKeyOwner::<T>::iter() {
            ensure!(
                AttestorPeerKeys::<T>::get(chain, controller) == Some(key),
                "orphaned peer key owner"
            );
        }
        for (chain, required) in PeerRegistrationRequired::<T>::iter() {
            if required {
                for controller in ActiveAttestors::<T>::get(chain) {
                    ensure!(
                        AttestorPeerKeys::<T>::contains_key(chain, controller),
                        "active committee is missing a required peer key"
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::*;
    use frame_support::{assert_noop, assert_ok};
    use sp_core::{ed25519, Pair, H256};

    fn pair(seed: u8) -> ed25519::Pair {
        ed25519::Pair::from_seed(&[seed; 32])
    }

    fn proof(controller: AccountId, key: &ed25519::Pair) -> [u8; 64] {
        let message = peer_key_proof_message(
            System::block_hash(0).as_ref(),
            SUPPORTED_CHAIN_KEY,
            &controller.encode(),
            &key.public().0,
        );
        key.sign(&message).0
    }

    fn register(stash: AccountId, controller: AccountId) {
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(stash),
            SUPPORTED_CHAIN_KEY,
            controller
        ));
    }

    fn set_key(controller: AccountId, key: &ed25519::Pair) {
        assert_ok!(Attestation::set_attestor_peer_key(
            RuntimeOrigin::signed(controller),
            SUPPORTED_CHAIN_KEY,
            key.public().0,
            proof(controller, key),
            AttestorPeerKeys::<Test>::get(SUPPORTED_CHAIN_KEY, controller),
        ));
    }

    #[test]
    fn controller_registers_and_idempotently_reasserts_its_peer_key() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            let key = pair(1);
            set_key(ATTESTOR_1, &key);
            assert_eq!(
                AttestorPeerKeys::<Test>::get(SUPPORTED_CHAIN_KEY, ATTESTOR_1),
                Some(key.public().0)
            );
            assert_eq!(
                PeerKeyOwner::<Test>::get(SUPPORTED_CHAIN_KEY, key.public().0),
                Some(ATTESTOR_1)
            );
            let events = System::events().len();
            set_key(ATTESTOR_1, &key);
            assert_eq!(System::events().len(), events);
        });
    }

    #[test]
    fn stash_and_unsigned_callers_cannot_register_a_controllers_key() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            let key = pair(1);
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(STASH_1),
                    SUPPORTED_CHAIN_KEY,
                    key.public().0,
                    proof(ATTESTOR_1, &key),
                    None,
                ),
                Error::<Test>::AddressNotAttestor
            );
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::none(),
                    SUPPORTED_CHAIN_KEY,
                    key.public().0,
                    proof(ATTESTOR_1, &key),
                    None,
                ),
                sp_runtime::DispatchError::BadOrigin
            );
        });
    }

    #[test]
    fn copied_proof_cannot_be_replayed_by_a_different_controller() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            register(STASH_2, ATTESTOR_2);
            let key = pair(1);
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(ATTESTOR_2),
                    SUPPORTED_CHAIN_KEY,
                    key.public().0,
                    proof(ATTESTOR_1, &key),
                    None,
                ),
                Error::<Test>::InvalidPeerKeyProof
            );
        });
    }

    #[test]
    fn proof_is_bound_to_genesis_source_chain_and_public_key() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            let key = pair(1);
            for (genesis, chain, public) in [
                (H256::repeat_byte(9), SUPPORTED_CHAIN_KEY, key.public().0),
                (
                    System::block_hash(0),
                    SUPPORTED_CHAIN_KEY + 1,
                    key.public().0,
                ),
                (
                    System::block_hash(0),
                    SUPPORTED_CHAIN_KEY,
                    pair(2).public().0,
                ),
            ] {
                let message =
                    peer_key_proof_message(genesis.as_ref(), chain, &ATTESTOR_1.encode(), &public);
                assert_noop!(
                    Attestation::set_attestor_peer_key(
                        RuntimeOrigin::signed(ATTESTOR_1),
                        SUPPORTED_CHAIN_KEY,
                        key.public().0,
                        key.sign(&message).0,
                        None,
                    ),
                    Error::<Test>::InvalidPeerKeyProof
                );
            }
        });
    }

    #[test]
    fn same_peer_key_cannot_be_owned_by_two_controllers_on_one_chain() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            register(STASH_2, ATTESTOR_2);
            let key = pair(1);
            set_key(ATTESTOR_1, &key);
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(ATTESTOR_2),
                    SUPPORTED_CHAIN_KEY,
                    key.public().0,
                    proof(ATTESTOR_2, &key),
                    None,
                ),
                Error::<Test>::PeerKeyAlreadyRegistered
            );
        });
    }

    #[test]
    fn rotation_updates_both_indexes_and_releases_the_old_identity() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            register(STASH_2, ATTESTOR_2);
            let old = pair(1);
            let new = pair(2);
            set_key(ATTESTOR_1, &old);
            set_key(ATTESTOR_1, &new);
            assert_eq!(
                AttestorPeerKeys::<Test>::get(SUPPORTED_CHAIN_KEY, ATTESTOR_1),
                Some(new.public().0)
            );
            assert!(!PeerKeyOwner::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                old.public().0
            ));
            set_key(ATTESTOR_2, &old);
        });
    }

    #[test]
    fn failed_rotation_keeps_the_old_registration_intact() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            let old = pair(1);
            set_key(ATTESTOR_1, &old);
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    pair(2).public().0,
                    proof(ATTESTOR_1, &old),
                    Some(old.public().0),
                ),
                Error::<Test>::InvalidPeerKeyProof
            );
            assert_eq!(
                AttestorPeerKeys::<Test>::get(SUPPORTED_CHAIN_KEY, ATTESTOR_1),
                Some(old.public().0)
            );
            assert_eq!(
                PeerKeyOwner::<Test>::get(SUPPORTED_CHAIN_KEY, old.public().0),
                Some(ATTESTOR_1)
            );
        });
    }

    #[test]
    fn stale_registration_cannot_overwrite_a_newer_key() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            let first = pair(1);
            let second = pair(2);
            let third = pair(3);
            set_key(ATTESTOR_1, &first);
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    second.public().0,
                    proof(ATTESTOR_1, &second),
                    None,
                ),
                Error::<Test>::PeerKeyRegistrationChanged
            );
            set_key(ATTESTOR_1, &second);
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    third.public().0,
                    proof(ATTESTOR_1, &third),
                    Some(first.public().0),
                ),
                Error::<Test>::PeerKeyRegistrationChanged
            );
            assert_eq!(
                AttestorPeerKeys::<Test>::get(SUPPORTED_CHAIN_KEY, ATTESTOR_1),
                Some(second.public().0)
            );
            // A lost RPC response may cause the original registration to be retried.
            assert_ok!(Attestation::set_attestor_peer_key(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                second.public().0,
                proof(ATTESTOR_1, &second),
                Some(first.public().0),
            ));
        });
    }

    #[test]
    fn unregistration_cleans_both_peer_key_indexes() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            let key = pair(1);
            set_key(ATTESTOR_1, &key);
            assert_ok!(Attestation::unregister_attestor(
                RuntimeOrigin::signed(STASH_1),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));
            assert!(!AttestorPeerKeys::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));
            assert!(!PeerKeyOwner::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                key.public().0
            ));
        });
    }

    #[test]
    fn rollout_cannot_require_keys_until_active_and_waiting_members_backfill() {
        ExtBuilder.build_and_execute(|| {
            assert!(!PeerRegistrationRequired::<Test>::get(SUPPORTED_CHAIN_KEY));
            register(STASH_3, ATTESTOR_3);
            Attestors::<Test>::mutate(SUPPORTED_CHAIN_KEY, ATTESTOR_3, |attestor| {
                attestor.as_mut().unwrap().status = attestor_primitives::AttestorStatus::Active;
            });
            ActiveAttestors::<Test>::insert(SUPPORTED_CHAIN_KEY, vec![ATTESTOR_3]);
            assert_noop!(
                Attestation::set_peer_registration_required(
                    RuntimeOrigin::root(),
                    SUPPORTED_CHAIN_KEY,
                    true
                ),
                Error::<Test>::PeerKeyNotRegistered
            );
            set_key(ATTESTOR_3, &pair(3));
            let attestor = crate::tests::Attestor::new(STASH_1, ATTESTOR_1);
            register(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::attest(
                attestor.attestor_origin,
                SUPPORTED_CHAIN_KEY,
                attestor.public_key,
                attestor.signature
            ));
            assert_noop!(
                Attestation::set_peer_registration_required(
                    RuntimeOrigin::root(),
                    SUPPORTED_CHAIN_KEY,
                    true
                ),
                Error::<Test>::PeerKeyNotRegistered
            );
            set_key(ATTESTOR_1, &pair(1));
            assert_ok!(Attestation::set_peer_registration_required(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                true
            ));
        });
    }

    #[test]
    fn required_registration_gates_new_activation_but_allows_idle_backfill() {
        ExtBuilder.build_and_execute(|| {
            let attestor = crate::tests::Attestor::new(STASH_1, ATTESTOR_1);
            register(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::set_peer_registration_required(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                true
            ));
            assert_noop!(
                Attestation::attest(
                    attestor.attestor_origin.clone(),
                    SUPPORTED_CHAIN_KEY,
                    attestor.public_key,
                    attestor.signature
                ),
                Error::<Test>::PeerKeyNotRegistered
            );
            set_key(ATTESTOR_1, &pair(1));
            assert_ok!(Attestation::attest(
                attestor.attestor_origin,
                SUPPORTED_CHAIN_KEY,
                attestor.public_key,
                attestor.signature
            ));
        });
    }

    #[test]
    fn only_operators_can_change_the_registration_policy() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::set_peer_registration_required(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    false
                ),
                sp_runtime::DispatchError::BadOrigin
            );
        });
    }

    #[test]
    fn registry_separates_waiting_candidates_from_voters_and_excludes_idle_keys() {
        ExtBuilder.build_and_execute(|| {
            register(STASH_1, ATTESTOR_1);
            set_key(ATTESTOR_1, &pair(1));
            ActiveAttestors::<Test>::insert(SUPPORTED_CHAIN_KEY, vec![ATTESTOR_1, ATTESTOR_3]);
            let registry = Attestation::attestor_peer_registry(SUPPORTED_CHAIN_KEY);
            assert_eq!(
                registry.attestors,
                vec![(ATTESTOR_1, Some(pair(1).public().0)), (ATTESTOR_3, None)]
            );
            assert!(!registry.registration_required);
            assert!(registry.waiting_attestors.is_empty());

            register(STASH_2, ATTESTOR_2);
            set_key(ATTESTOR_2, &pair(2));
            // Idle key backfill does not grant admission.
            assert!(Attestation::attestor_peer_registry(SUPPORTED_CHAIN_KEY)
                .waiting_attestors
                .is_empty());
            Attestors::<Test>::mutate(SUPPORTED_CHAIN_KEY, ATTESTOR_2, |attestor| {
                attestor.as_mut().unwrap().status = AttestorStatus::Waiting;
            });
            let waiting = Attestation::attestor_peer_registry(SUPPORTED_CHAIN_KEY);
            assert_eq!(waiting.attestors, registry.attestors);
            assert_eq!(
                waiting.waiting_attestors,
                vec![(ATTESTOR_2, Some(pair(2).public().0))]
            );
            for policy in [
                AttestorElectionPolicy::DeniedToAll,
                AttestorElectionPolicy::AuthorizedOnly,
            ] {
                ChainElectionPolicy::<Test>::insert(SUPPORTED_CHAIN_KEY, policy);
                assert!(Attestation::attestor_peer_registry(SUPPORTED_CHAIN_KEY)
                    .waiting_attestors
                    .is_empty());
            }
            AuthorizedAttestors::<Test>::insert(SUPPORTED_CHAIN_KEY, ATTESTOR_2, ());
            assert_eq!(
                Attestation::attestor_peer_registry(SUPPORTED_CHAIN_KEY).waiting_attestors,
                waiting.waiting_attestors
            );
            ActiveAttestors::<Test>::mutate(SUPPORTED_CHAIN_KEY, |active| active.push(ATTESTOR_2));
            assert!(Attestation::attestor_peer_registry(SUPPORTED_CHAIN_KEY)
                .waiting_attestors
                .is_empty());
        });
    }

    #[test]
    fn removed_chain_drains_peer_keys_in_bounded_batches() {
        ExtBuilder.build_and_execute(|| {
            let count = crate::clear_or_revert::MAX_REMOVAL_CLEANUP_ITEMS_PER_BLOCK * 3;
            for i in 0..count {
                let controller = 10_000 + u64::from(i);
                let key = pair(i as u8).public().0;
                Attestors::<Test>::insert(
                    SUPPORTED_CHAIN_KEY,
                    controller,
                    attestor_primitives::Attestor {
                        bls_public_key: None,
                        status: attestor_primitives::AttestorStatus::Idle,
                        stash: STASH_1,
                    },
                );
                AttestorPeerKeys::<Test>::insert(SUPPORTED_CHAIN_KEY, controller, key);
                PeerKeyOwner::<Test>::insert(SUPPORTED_CHAIN_KEY, key, controller);
            }
            PeerRegistrationRequired::<Test>::insert(SUPPORTED_CHAIN_KEY, true);
            assert_ok!(SupportedChains::remove_chain(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                false
            ));
            assert!(!PeerRegistrationRequired::<Test>::get(SUPPORTED_CHAIN_KEY));
            for _ in 0..20 {
                let before = AttestorPeerKeys::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count();
                Attestation::on_init_cleanup_removed_chains();
                let after = AttestorPeerKeys::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count();
                assert!(
                    before.saturating_sub(after)
                        <= crate::clear_or_revert::MAX_REMOVAL_CLEANUP_ITEMS_PER_BLOCK as usize
                );
            }
            assert_eq!(
                AttestorPeerKeys::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
                0
            );
            assert_eq!(
                PeerKeyOwner::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
                0
            );
            assert!(!RemovedChainCleanup::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY
            ));
            assert_noop!(
                Attestation::set_attestor_peer_key(
                    RuntimeOrigin::signed(ATTESTOR_3),
                    SUPPORTED_CHAIN_KEY,
                    pair(3).public().0,
                    proof(ATTESTOR_3, &pair(3)),
                    None,
                ),
                Error::<Test>::ChainNotSupported
            );
        });
    }
}
