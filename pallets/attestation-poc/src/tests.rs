use super::*;
use crate::mock::{
    Attestation, ExtBuilder, RuntimeOrigin, Test, ATTESTOR_1, ATTESTOR_2, DEFAULT_COMITTEE_SET_SIZE,
};
use assert_matches::assert_matches;
use attestor_primitives::{Attestation as AttestationPrimitive, ChainId, SignedAttestation};
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
fn register_attestor_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor,
            att.public_key,
            att.signature
        ));
    })
}

#[test]
fn register_attestor_should_fail_when_address_is_already_registered() {
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
fn register_attestor_should_fail_when_list_is_full() {
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
        assert_noop!(
            Attestation::register_attestor(att_2.attestor, att_2.public_key, att_2.signature),
            Error::<Test>::AttestorListFull
        );
    })
}

// TODO: make this smarter and rely on the runtime value instead of the function
#[test]
fn max_attestor_default_should_be_100() {
    ExtBuilder.build_and_execute(|| assert_matches!(Attestation::max_attestors(), 100))
}

#[test]
fn max_invulnerable_default_should_be_100() {
    ExtBuilder.build_and_execute(|| assert_matches!(Attestation::max_invulnerables(), 100))
}

#[test]
fn set_max_attestors_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(Attestation::set_max_attestors(bad_origin, 1), BadOrigin);
    })
}

#[test]
fn set_max_invulnerables_should_error_with_new_max_less_than_current_count() {
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
fn set_max_invulnerables_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::set_max_invulnerables(bad_origin, 200),
            BadOrigin
        );
    })
}

#[test]
fn set_max_attestors_should_error_if_list_is_truncated() {
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
        assert_noop!(
            Attestation::set_max_attestors(RuntimeOrigin::root(), 1),
            Error::<Test>::MaxAttestorsCannotBeChanged
        );
    })
}

#[test]
fn unregister_attestor_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor.clone(),
            att.public_key,
            att.signature
        ));
        assert_ok!(Attestation::unregister_attestor(att.attestor));
    })
}

#[test]
fn unregister_attestor_should_fail_when_address_is_not_registered() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::unregister_attestor(attestor),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn unregister_invulnerable_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor.clone(),
            att.public_key,
            att.signature
        ));

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            att.public_key
        ));
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1
        ));
    })
}

#[test]
fn unregister_invulnerable_should_fail_when_address_is_not_registered() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), ATTESTOR_1),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}
#[test]
fn unregister_invulnerable_should_fail_when_address_is_not_invulnerable() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.attestor.clone(),
            att.public_key,
            att.signature
        ));
        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), ATTESTOR_1),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}

#[test]
fn test_set_max_comittee_size_root_works() {
    ExtBuilder.build_and_execute(|| {
        let comittee_size = Attestation::comittee_set_size();
        assert_eq!(comittee_size, DEFAULT_COMITTEE_SET_SIZE);

        let new_comittee_size = 512;
        assert_ok!(Attestation::set_comittee_set_size(
            RuntimeOrigin::root(),
            new_comittee_size
        ));

        let comittee_size = Attestation::comittee_set_size();
        assert_eq!(comittee_size, new_comittee_size);
    })
}

#[test]
fn test_set_max_comittee_size_other_fails() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);

        assert_noop!(Attestation::set_comittee_set_size(attestor, 512), BadOrigin);
    })
}

#[test]
fn add_invulnerable_also_adds_as_attestor_works() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(ATTESTOR_1);
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            att.public_key
        ));

        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(Attestation::invulnerables(ATTESTOR_1))
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
fn setting_attestation_interval_works() {
    ExtBuilder.build_and_execute(|| {
        let attestation_interval = Attestation::chain_attestation_interval(1);
        assert_eq!(attestation_interval, 10); // Interval set in mock genesis

        let chain_id = 1;
        let chain_attestation_interval = 101;
        assert_ok!(Attestation::set_chain_attestation_interval(
            RuntimeOrigin::root(),
            chain_id,
            chain_attestation_interval
        ));

        let attestation_interval = Attestation::chain_attestation_interval(1);
        assert_eq!(attestation_interval, 101);
    })
}

#[test]
fn setting_attestation_interval_for_unsupported_chain_fails() {
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
fn bootstrapping_unsupported_chain_fails() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;

        let attestor = Attestor::new(ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.attestor.clone(),
            attestor.public_key,
            attestor.signature
        ));

        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::root(), chain_id, attestation,),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn submitting_attestation_works() {
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
            attestation
        ));
    })
}

#[test]
fn submitting_duplicate_attestation_fails() {
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
fn submitting_attestation_chain_works() {
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

        let digest = attestation.digest();

        let attestation = create_signed_attestation(vec![attestor], 1, 11, Some(digest));

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation
        ));
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
