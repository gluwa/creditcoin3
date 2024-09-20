use super::*;
use crate::mock::*;
use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestationCheckpoint, ChainId, SignedAttestation,
};
use attestor_primitives::{BlsPublicKey, BlsSignature};
use bls_signatures::{aggregate, key::Serialize, PrivateKey};
use frame_support::{assert_noop, assert_ok};
use sp_core::H256;
use sp_runtime::traits::BadOrigin;

#[derive(Debug, Clone)]
pub struct Attestor {
    pub id: mock::AccountId,
    pub attestor: RuntimeOrigin,
    private_key: PrivateKey,
    pub public_key: BlsPublicKey,
    pub signature: BlsSignature,
}

impl Attestor {
    pub fn new(attestor: u64) -> Self {
        let rng = sp_core::H256::random().0;
        let private_key = PrivateKey::new(rng);
        let public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();
        let signature = private_key.sign(public_key).as_bytes()[..]
            .try_into()
            .unwrap();

        let id = attestor;
        let attestor = RuntimeOrigin::signed(attestor);

        Self {
            id,
            attestor,
            private_key,
            public_key,
            signature,
        }
    }

    fn sign(&self, message: &[u8]) -> BlsSignature {
        self.private_key.sign(message).as_bytes()[..]
            .try_into()
            .unwrap()
    }

    fn private_key(&self) -> PrivateKey {
        self.private_key
    }
}

#[test]
fn register_attestor_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // 1 invulnerable from mock runtime
        assert_eq!(Attestors::<Test>::count(), 1);

        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor,
            att.public_key,
            att.signature
        ));

        assert_eq!(Attestors::<Test>::count(), 2);
        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(Attestation::is_attestor(&ATTESTOR_1));
        System::assert_last_event(crate::Event::AttestorRegistered(ATTESTOR_1).into());
    })
}

#[test]
fn register_attestor_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_noop!(
            Attestation::register_attestor(RuntimeOrigin::none(), att.public_key, att.signature),
            BadOrigin
        );
    })
}

#[test]
fn register_attestor_should_error_when_address_is_already_registered() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor.clone(),
            att.public_key,
            att.signature
        ));

        assert_noop!(
            Attestation::register_attestor(att.attestor, att.public_key, att.signature),
            Error::<Test>::AlreadyAttestor
        );
    })
}

#[test]
fn register_attestor_should_error_when_public_key_is_invalid() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_noop!(
            Attestation::register_attestor(
                att.attestor,
                *b"000000000000000000000000000000000000000000000000",
                att.signature
            ),
            Error::<Test>::InvalidBlsPublicKey
        );
    })
}

#[test]
fn register_attestor_should_error_when_signature_is_invalid() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);

        assert_noop!(
            Attestation::register_attestor(
                att.attestor,
                att.public_key,
                *b"000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            ),
            Error::<Test>::InvalidBlsSignature
        );
    })
}

#[test]
fn register_attestor_should_error_when_signature_doesnt_validate_against_public_key() {
    ExtBuilder.build_and_execute(|| {
        let att1 = Attestor::new(ATTESTOR_1);
        let att2 = Attestor::new(ATTESTOR_2);

        assert_noop!(
            Attestation::register_attestor(att1.attestor, att1.public_key, att2.signature),
            Error::<Test>::InvalidProofOfPossession
        );
    })
}

#[test]
fn register_attestor_should_error_when_list_is_full() {
    ExtBuilder.build_and_execute(|| {
        let root = RuntimeOrigin::root();
        let att_1 = Attestor::new(ATTESTOR_1);
        let att_2 = Attestor::new(ATTESTOR_2);
        assert_ok!(Attestation::set_max_attestors(root, 2));
        assert_ok!(Attestation::register_attestor(
            att_1.attestor,
            att_1.public_key,
            att_1.signature
        ));

        // note: test target is try_insert_attestor_and_emit_event()
        assert_noop!(
            Attestation::register_attestor(att_2.attestor, att_2.public_key, att_2.signature),
            Error::<Test>::AttestorListFull
        );
    })
}

// TODO: make this smarter and rely on the runtime value instead of the function
#[test]
fn max_attestor_default_should_be_100() {
    ExtBuilder.build_and_execute(|| assert_eq!(Attestation::max_attestors(), 100))
}

#[test]
fn max_invulnerable_default_should_be_100() {
    ExtBuilder.build_and_execute(|| assert_eq!(Attestation::max_invulnerables(), 100))
}

#[test]
fn set_max_invulnerables_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_max_invulnerables(RuntimeOrigin::none(), 200),
            BadOrigin
        );
    })
}

#[test]
fn set_max_invulnerables_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::set_max_invulnerables(bad_origin, 200),
            BadOrigin
        );
    })
}

#[test]
fn set_max_invulnerables_should_error_when_value_is_less_than_current_count() {
    ExtBuilder.build_and_execute(|| {
        let root_origin = RuntimeOrigin::root();
        // There should be at least one invulnerable set in mock.rs
        // We set the max number of invulnerables to 0, less than the current number.
        assert_noop!(
            Attestation::set_max_invulnerables(root_origin, 0),
            Error::<Test>::MaxInvulnerablesCannotBeChanged
        );
    })
}

#[test]
fn set_max_invulnerables_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        assert_eq!(Attestation::max_invulnerables(), 100);
        assert_eq!(Invulnerables::<Test>::count(), 1); // from mock

        assert_ok!(Attestation::set_max_invulnerables(
            RuntimeOrigin::root(),
            10
        ),);
        assert_eq!(Attestation::max_invulnerables(), 10);
        assert_eq!(Invulnerables::<Test>::count(), 1); // this wasn't updated
    })
}

#[test]
fn set_max_attestors_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_max_attestors(RuntimeOrigin::none(), 1),
            BadOrigin
        );
    })
}

#[test]
fn set_max_attestors_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(Attestation::set_max_attestors(bad_origin, 1), BadOrigin);
    })
}

#[test]
fn set_max_attestors_should_work_when_truncating_existing_list() {
    ExtBuilder.build_and_execute(|| {
        let att_1 = Attestor::new(ATTESTOR_1);
        let att_2 = Attestor::new(ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att_1.attestor,
            att_1.public_key,
            att_1.signature
        ));
        assert_ok!(Attestation::register_attestor(
            att_2.attestor,
            att_2.public_key,
            att_2.signature
        ));

        // 1 invulnerable coming from mock runtime
        let count = Attestors::<Test>::count();
        assert_eq!(count, 3);

        assert_ok!(Attestation::set_max_attestors(RuntimeOrigin::root(), 1));
        let count = Attestors::<Test>::count();
        assert_eq!(count, 3);
        let max_attestors = Attestation::max_attestors();
        assert_eq!(max_attestors, 1);
    })
}

#[test]
fn set_max_attestors_should_work_when_list_is_empty() {
    ExtBuilder.build_and_execute(|| {
        let _ = Attestors::<Test>::clear(u32::MAX, None);
        let count = Attestors::<Test>::count();
        assert_eq!(count, 0);

        assert_ok!(Attestation::set_max_attestors(RuntimeOrigin::root(), 5));
        let max_attestors = Attestation::max_attestors();
        assert_eq!(max_attestors, 5);
    })
}

#[test]
fn set_max_attestors_should_work_when_expanding_existing_list() {
    ExtBuilder.build_and_execute(|| {
        let att_1 = Attestor::new(ATTESTOR_1);
        let att_2 = Attestor::new(ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att_1.attestor,
            att_1.public_key,
            att_1.signature
        ));
        assert_ok!(Attestation::register_attestor(
            att_2.attestor,
            att_2.public_key,
            att_2.signature
        ));

        // 1 invulnerable coming from mock runtime
        let count = Attestors::<Test>::count();
        assert_eq!(count, 3);
        // this is the default value
        let max_attestors = Attestation::max_attestors();
        assert_eq!(max_attestors, 100);

        assert_ok!(Attestation::set_max_attestors(RuntimeOrigin::root(), 10),);
        let max_attestors = Attestation::max_attestors();
        assert_eq!(max_attestors, 10);
    })
}

#[test]
fn unregister_attestor_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_attestor(RuntimeOrigin::none()),
            BadOrigin
        );
    })
}

#[test]
fn unregister_attestor_should_error_when_address_is_not_registered_as_attestor() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::unregister_attestor(attestor),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn unregister_attestor_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // setup
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor.clone(),
            att.public_key,
            att.signature
        ));
        assert!(Attestation::is_attestor(&ATTESTOR_1));

        // test
        assert_ok!(Attestation::unregister_attestor(att.attestor.clone()));
        assert!(!Attestation::is_attestor(&ATTESTOR_1));
        System::assert_last_event(crate::Event::AttestorUnregistered(ATTESTOR_1).into());
    })
}

#[test]
fn unregister_invulnerable_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // setup
        assert!(!Attestors::<Test>::contains_key(ATTESTOR_1));
        assert!(!Invulnerables::<Test>::contains_key(ATTESTOR_1));

        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            att.public_key
        ));
        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(Attestation::invulnerables(ATTESTOR_1));

        // test
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1
        ));
        assert!(!Attestors::<Test>::contains_key(ATTESTOR_1));
        assert!(!Invulnerables::<Test>::contains_key(ATTESTOR_1));
        System::assert_last_event(crate::Event::InvulnerableUnregistered(ATTESTOR_1).into())
    })
}

#[test]
fn unregister_invulnerable_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::none(), ATTESTOR_1),
            BadOrigin
        );
    })
}

#[test]
fn unregister_invulnerable_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::signed(ATTESTOR_1), ATTESTOR_1),
            BadOrigin
        );
    })
}

#[test]
fn unregister_invulnerable_should_fail_when_address_is_not_registered_at_all() {
    ExtBuilder.build_and_execute(|| {
        assert!(!Attestors::<Test>::contains_key(ATTESTOR_1));
        assert!(!Invulnerables::<Test>::contains_key(ATTESTOR_1));

        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), ATTESTOR_1),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}

#[test]
fn unregister_invulnerable_should_fail_when_address_is_an_attestor_but_not_invulnerable() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor.clone(),
            att.public_key,
            att.signature
        ));
        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(!Invulnerables::<Test>::contains_key(ATTESTOR_1));

        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), ATTESTOR_1),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}

#[test]
fn set_comittee_set_size_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            Attestation::set_comittee_set_size(RuntimeOrigin::none(), 512),
            BadOrigin
        );
    })
}

#[test]
fn set_comittee_set_size_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);

        assert_noop!(Attestation::set_comittee_set_size(attestor, 512), BadOrigin);
    })
}

#[test]
fn set_comittee_set_size_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let comittee_size = Attestation::comittee_set_size();
        assert_eq!(comittee_size, DEFAULT_COMITTEE_SET_SIZE);

        let new_comittee_size = 512;
        assert_ok!(Attestation::set_comittee_set_size(
            RuntimeOrigin::root(),
            new_comittee_size
        ));

        let comittee_size = Attestation::comittee_set_size();
        assert_eq!(comittee_size, new_comittee_size);

        System::assert_last_event(crate::Event::ComitteeSetSizeChanged(new_comittee_size).into());
    })
}

#[test]
fn register_invulnerable_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);

        assert_noop!(
            Attestation::register_invulnerable(RuntimeOrigin::none(), ATTESTOR_1, att.public_key),
            BadOrigin
        );
    })
}

#[test]
fn register_invulnerable_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_noop!(
            Attestation::register_invulnerable(
                RuntimeOrigin::signed(ATTESTOR_1),
                ATTESTOR_1,
                att.public_key
            ),
            BadOrigin
        );
    })
}

#[test]
fn register_invulnerable_adds_attestor_and_invulnerable_and_emits_events() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert!(!Attestors::<Test>::contains_key(ATTESTOR_1));
        assert!(!Invulnerables::<Test>::contains_key(ATTESTOR_1));

        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            att.public_key
        ));

        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(Attestation::invulnerables(ATTESTOR_1));

        // assert on event
        System::assert_last_event(crate::Event::InvulnerableRegistered(ATTESTOR_1).into());
    })
}

// Rare case that an invulnerable signals unregister and then sudo removes that one as invulnerable
#[test]
fn remove_invulnerable_that_is_not_attestor_works() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            att.public_key
        ));

        // Unregister
        assert_ok!(Attestation::unregister_attestor(att.attestor));

        // Not an attestor anymore
        assert!(Attestation::attestors(ATTESTOR_1).is_none());

        // Still invulnerable
        assert!(Attestation::invulnerables(ATTESTOR_1));

        // Remove as invulnerable
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1
        ));
    })
}

#[test]
fn set_chain_attestation_interval_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_attestation_interval = 101;

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::none(),
                chain_id,
                chain_attestation_interval
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_attestation_interval_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_attestation_interval = 101;

        let acct: AccountId = 4;

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::signed(acct),
                chain_id,
                chain_attestation_interval
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_attestation_interval_should_error_for_unsupported_chain() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let chain_attestation_interval = 101;
        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::root(),
                chain_id,
                chain_attestation_interval
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn set_chain_attestation_interval_updates_internal_storage() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 1;

        let attestation_interval = Attestation::chain_attestation_interval(chain_id);
        assert_eq!(attestation_interval, 10); // Interval set in mock genesis

        let chain_attestation_interval = 101;
        assert_ok!(Attestation::set_chain_attestation_interval(
            RuntimeOrigin::root(),
            chain_id,
            chain_attestation_interval
        ));

        let attestation_interval = Attestation::chain_attestation_interval(chain_id);
        assert_eq!(attestation_interval, 101);
    })
}

#[test]
fn set_attestations_per_checkpoint_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 1;
        let att_per_check = Attestation::attestation_checkpoint_interval(chain_id);
        assert_eq!(att_per_check, 10); // Checkpoint frequencty set in mock genesis

        let new_att_per_check = 101;
        assert_ok!(Attestation::set_attestations_per_checkpoint(
            RuntimeOrigin::root(),
            chain_id,
            new_att_per_check
        ));

        let att_per_check = Attestation::attestation_checkpoint_interval(chain_id);
        assert_eq!(att_per_check, 101);
    })
}

#[test]
fn set_attestations_per_checkpoint_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_attestations_per_checkpoint(RuntimeOrigin::none(), 2, 101),
            BadOrigin
        );
    })
}

#[test]
fn set_attestations_per_checkpoint_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_attestations_per_checkpoint(RuntimeOrigin::signed(ATTESTOR_1), 2, 101),
            BadOrigin
        );
    })
}

#[test]
fn set_attestations_per_checkpoint_should_error_on_unsupported_chain() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let att_per_check = 101;
        assert_noop!(
            Attestation::set_attestations_per_checkpoint(
                RuntimeOrigin::root(),
                chain_id,
                att_per_check
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::none(), chain_id, attestation,),
            BadOrigin
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::signed(ATTESTOR_1), chain_id, attestation,),
            BadOrigin
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_chain_is_unsupported() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::root(), chain_id, attestation,),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn bootstrap_chain_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_id = 1; // supported in mock runtime
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_eq!(Attestation::last_attestation_digest(chain_id), None);
        assert_eq!(
            Attestation::attestations(chain_id, attestation.digest()),
            None
        );
        assert_eq!(Attestation::checkpointing_queues(chain_id).len(), 0);

        assert_ok!(Attestation::bootstrap_chain(
            RuntimeOrigin::root(),
            chain_id,
            attestation.clone(),
        ),);

        // storage
        assert_eq!(
            Attestation::last_attestation_digest(chain_id),
            Some(attestation.digest())
        );
        assert_eq!(
            Attestation::attestations(chain_id, attestation.digest()),
            Some(attestation.clone())
        );
        // Shouldn't add first attestation for chain to checkpointing queue
        assert_eq!(Attestation::checkpointing_queues(chain_id).len(), 0);

        // event
        let expected_digest = attestation.digest();
        System::assert_last_event(
            crate::Event::BlockAttested(chain_id, attestation, expected_digest).into(),
        );
    })
}

#[test]
fn commit_attestation_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);
        let chain_id = 1;
        assert_eq!(Attestation::checkpointing_queues(chain_id).len(), 0);

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let attestation = create_signed_attestation(vec![attestor], chain_id, 1, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        // The first attestation for a chain immediately creates a corresponding checkpoint
        // rather than adding to the checkpointing queue.
        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation.header_number(),
            digest: attestation.digest(),
            prev_digest: attestation.attestation.prev_digest,
        };
        assert_eq!(
            Attestation::checkpoints(chain_id, expected_checkpoint.digest),
            Some(expected_checkpoint)
        );

        assert_eq!(
            Attestation::attestations(chain_id, attestation.digest()),
            Some(attestation)
        );
    })
}

#[test]
fn commit_attestation_should_error_when_signed() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 1, 1, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::signed(ATTESTOR_1), attestation),
            BadOrigin
        );
    })
}

#[test]
fn commit_attestation_should_error_when_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 1, 1, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::root(), attestation),
            BadOrigin
        );
    })
}

#[test]
fn commit_attestation_should_error_when_chain_is_not_supported() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;

        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], chain_id, 1, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn commit_attestation_should_error_when_submitting_duplicate_attestation() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let attestation = create_signed_attestation(vec![attestor], 1, 1, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::AttestationExists
        );
    })
}

#[test]
fn commit_attestation_should_error_when_it_cannot_validate_the_attestation() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 1, 1, None);

        // note: not calling register_attestor() will cause the validation to fail
        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::InvalidAttestation
        );
    })
}

#[test]
fn submitting_attestation_chain_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);
        let chain_id = 1;
        assert_eq!(Attestation::checkpointing_queues(chain_id).len(), 0);

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let attestation_1 = create_signed_attestation(vec![attestor.clone()], chain_id, 1, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation_1.clone()
        ));

        let digest = attestation_1.digest();

        let attestation_2 = create_signed_attestation(vec![attestor], chain_id, 11, Some(digest));

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation_2.clone()
        ));

        // Only second attestation should have been added to a queue
        assert_eq!(Attestation::checkpointing_queues(chain_id).len(), 1);
        assert_eq!(
            Attestation::checkpointing_queues(chain_id).back(),
            Some(&attestation_2.digest())
        );
        assert_eq!(
            Attestation::attestations(chain_id, attestation_1.digest()),
            Some(attestation_1)
        );
        assert_eq!(
            Attestation::attestations(chain_id, attestation_2.digest()),
            Some(attestation_2)
        );
    })
}

#[test]
fn submitting_invalid_attestation_chain_fails() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let attestation = create_signed_attestation(vec![attestor.clone()], 1, 1, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        let fake_digest = H256::random();

        let attestation =
            create_signed_attestation(vec![attestor.clone()], 1, 11, Some(fake_digest));

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::InvalidAttestation
        );
    });
}

#[test]
fn invalid_proof_of_possession() {
    ExtBuilder.build_and_execute(|| {
        let att_1 = Attestor::new(ATTESTOR_1);
        let att_2 = Attestor::new(ATTESTOR_2);
        assert_noop!(
            Attestation::register_attestor(att_1.attestor, att_1.public_key, att_2.signature),
            Error::<Test>::InvalidProofOfPossession
        );
    })
}

#[test]
fn test_signing() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        let message = att.public_key;
        let signature = att.sign(message[..].try_into().unwrap());
        assert!(att.private_key().public_key().verify(
            bls_signatures::Signature::from_bytes(&signature[..]).unwrap(),
            message
        ));
    })
}

#[test]
fn creating_checkpoint_works() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        // Setup almost two full checkpoints of attestations, so that
        // the next attestation submitted triggers checkpoint creation.
        let attestor = Attestor::new(ATTESTOR_1);
        let chain_id = 1;
        let att_interval = Attestation::chain_attestation_interval(chain_id);
        let att_per_check = Attestation::attestation_checkpoint_interval(chain_id);

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let mut last_digest: Option<H256> = None;
        let mut removed_by_checkpoint: Vec<H256> = Vec::new();
        let mut kept_after_checkpoint: Vec<SignedAttestation<H256, u64>> = Vec::new();
        let mut checkpoint_attestation: Option<SignedAttestation<H256, u64>> = None;
        for i in 0..(att_per_check * 2 + 1) as usize {
            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                chain_id,
                (att_interval * i as u64) + 1,
                last_digest,
            );
            last_digest = Some(attestation.digest());

            assert_ok!(Attestation::commit_attestation(
                RuntimeOrigin::none(),
                attestation.clone()
            ));

            match i {
                i if i < att_per_check as usize && i != 0 => {
                    removed_by_checkpoint.push(attestation.digest());
                }
                i if i == att_per_check as usize => {
                    // End of first checkpoint interval
                    removed_by_checkpoint.push(attestation.digest());
                    checkpoint_attestation = Some(attestation);
                }
                _ => {
                    kept_after_checkpoint.push(attestation);
                }
            }
        }

        assert_eq!(
            Attestation::checkpointing_queues(chain_id).len(),
            att_per_check as usize
        );

        // TODO: Uncomment this once removing attestations from storage is enabled again.
        // This will happen when checkpoints are fully implemented.
        /*
        for removed_digest in removed_by_checkpoint {
            assert_eq!(Attestation::attestations(chain_id, removed_digest), None);
        }
        */

        for kept_attestation in kept_after_checkpoint {
            assert_eq!(
                Attestation::attestations(chain_id, kept_attestation.digest()),
                Some(kept_attestation)
            )
        }

        let unwrapped_att =
            checkpoint_attestation.expect("Should have been filled to Some in loop.");
        let resulting_checkpoint = AttestationCheckpoint {
            block_number: unwrapped_att.header_number(),
            digest: unwrapped_att.digest(),
            prev_digest: unwrapped_att.attestation.prev_digest,
        };
        System::assert_last_event(
            crate::Event::CheckpointReached(chain_id, resulting_checkpoint.clone()).into(),
        );
        assert_eq!(
            Attestation::checkpoints(chain_id, resulting_checkpoint.digest),
            Some(resulting_checkpoint)
        )
    })
}

#[test]
fn checkpointing_rolls_back_storage_changes_if_checkpointing_queue_does_not_match_attestations_map()
{
    ExtBuilder.build_and_execute(|| {
        // Needed to emit event
        System::set_block_number(1);
        // Setup almost two full checkpoints of attestations, so that
        // the next attestation submitted triggers checkpoint creation.
        let attestor = Attestor::new(ATTESTOR_1);
        let chain_id = 1;
        let att_interval = Attestation::chain_attestation_interval(chain_id);
        let att_per_check = Attestation::attestation_checkpoint_interval(chain_id) as u64;

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let mut last_digest: Option<H256> = None;
        let mut attestations = Vec::new();
        // Add initial attestation, which belongs to its own special checkpoint interval,
        // as well as all but 1 of the attestations in the following interval.
        for i in 0..att_per_check {
            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                chain_id,
                (att_interval * i) + 1,
                last_digest,
            );
            last_digest = Some(attestation.digest());
            attestations.push(attestation.clone());

            assert_ok!(Attestation::commit_attestation(
                RuntimeOrigin::none(),
                attestation.clone()
            ));
        }

        // Inserts a garbage checkpointing queue entry without corresponding
        // attestations entry. We break checkpointing part way through,
        // requiring that all previous state changes be rolled back.
        Attestation::break_checkpointing();

        // Trigger checkpointing by adding one more full interval of attestations
        for i in (att_per_check)..(att_per_check * 2) {
            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                chain_id,
                (att_interval * i) + 1,
                last_digest,
            );
            last_digest = Some(attestation.digest());

            // Final attestation
            if i == att_per_check * 2 {
                // Before committing final attestation, queue should contain 2
                // checkpoints worth of attestations - 1
                assert_eq!(
                    Attestation::checkpointing_queues(chain_id).len(),
                    (att_per_check * 2 - 1) as usize
                );

                assert_ok!(Attestation::commit_attestation(
                    RuntimeOrigin::none(),
                    attestation.clone()
                ));

                // The final attestation should have been successfully added to
                // the queue, and then any removals from the queue due to
                // checkpointing should have been rolled back.
                assert_eq!(
                    Attestation::checkpointing_queues(chain_id).len(),
                    (att_per_check * 2) as usize
                );

                // Check that no attestations are missing from storage
                for attestation in &attestations {
                    assert_eq!(
                        Attestation::attestations(chain_id, attestation.digest()),
                        Some(attestation.clone())
                    );
                }
            } else {
                // No checkpointing this pass
                assert_ok!(Attestation::commit_attestation(
                    RuntimeOrigin::none(),
                    attestation.clone()
                ));
            }
        }
    })
}

fn create_signed_attestation(
    attestors: Vec<Attestor>,
    chain_id: ChainId,
    header_number: u64,
    prev_digest: Option<H256>,
) -> SignedAttestation<H256, mock::AccountId> {
    let attestation = AttestationPrimitive {
        chain_id,
        header_number,
        header_hash: H256::random(),
        root: [0; 32],
        prev_digest,
    };

    let mut signatures = Vec::new();
    for attestor in attestors.iter() {
        let signature = attestor.sign(&attestation.serialize());
        let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
            .expect("Failed to create signature");

        signatures.push(bls_sig);
    }
    // sign
    let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

    let attestation = SignedAttestation {
        attestation,
        signature: aggregated_signature.as_bytes()[..]
            .try_into()
            .expect("Failed to convert to array"),
        attestors: attestors.iter().map(|a| a.id).collect::<Vec<_>>(),
    };

    attestation
}
