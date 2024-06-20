use super::*;
use crate::mock::{
    Attestation, ExtBuilder, RuntimeOrigin, Test, ATTESTOR_1, ATTESTOR_2, DEFAULT_COMITTEE_SET_SIZE,
};
use assert_matches::assert_matches;
use attestor_primitives::{Attestation as AttestationPrimitive, SignedAttestation};
use attestor_primitives::{BlsPublicKey, BlsSignature};
use bls_signatures::key::Serialize;
use frame_support::{assert_err, assert_ok};
use sp_core::H256;
use sp_runtime::traits::BadOrigin;

pub fn attestor(attestor: u64) -> (RuntimeOrigin, BlsPublicKey, BlsSignature) {
    let rng = sp_core::H256::random().0;
    let bls_private_key = bls_signatures::PrivateKey::new(rng);
    let bls_public_key = bls_private_key.public_key().as_bytes()[..]
        .try_into()
        .unwrap();

    (
        RuntimeOrigin::signed(attestor),
        bls_public_key,
        bls_private_key.sign(bls_public_key).as_bytes()[..]
            .try_into()
            .unwrap(),
    )
}

#[test]
fn register_attestor_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let (attestor_1, public_key, signature) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor_1, public_key, signature
        ));
    })
}

#[test]
fn register_attestor_should_fail_when_address_is_already_registered() {
    ExtBuilder.build_and_execute(|| {
        let (attestor_1, public_key, signature) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor_1.clone(),
            public_key,
            signature
        ));

        assert_err!(
            Attestation::register_attestor(attestor_1, public_key, signature),
            Error::<Test>::AlreadyAttestor
        );
    })
}

#[test]
fn register_attestor_should_fail_when_list_is_full() {
    ExtBuilder.build_and_execute(|| {
        let root = RuntimeOrigin::root();
        let (attestor_1, public_key, signature) = attestor(ATTESTOR_1);
        let (attestor_2, public_key_2, signature_2) = attestor(ATTESTOR_2);

        assert_ok!(Attestation::set_max_attestors(root, 2));
        assert_ok!(Attestation::register_attestor(
            attestor_1, public_key, signature
        ));
        assert_err!(
            Attestation::register_attestor(attestor_2, public_key_2, signature_2),
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
        let bad_origin = attestor(ATTESTOR_1).0;
        assert_err!(Attestation::set_max_attestors(bad_origin, 1), BadOrigin)
    })
}

#[test]
fn set_max_invulnerables_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = attestor(ATTESTOR_1).0;
        assert_err!(
            Attestation::set_max_invulnerables(bad_origin, 200),
            BadOrigin
        )
    })
}

#[test]
fn set_max_attestors_should_error_if_list_is_truncated() {
    ExtBuilder.build_and_execute(|| {
        let (attestor_1, public_key_1, signature_1) = attestor(ATTESTOR_1);
        let (attestor_2, public_key_2, signature_2) = attestor(ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            attestor_1,
            public_key_1,
            signature_1
        ));
        assert_ok!(Attestation::register_attestor(
            attestor_2,
            public_key_2,
            signature_2
        ));
        assert_err!(
            Attestation::set_max_attestors(RuntimeOrigin::root(), 1),
            Error::<Test>::MaxAttestorsCannotBeChanged
        );
    })
}

#[test]
fn unregister_attestor_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let (attestor, public_key, signature) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.clone(),
            public_key,
            signature
        ));
        assert_ok!(Attestation::unregister_attestor(attestor));
    })
}

#[test]
fn unregister_attestor_should_fail_when_address_is_not_registered() {
    ExtBuilder.build_and_execute(|| {
        let attestor = attestor(ATTESTOR_1);
        assert_err!(
            Attestation::unregister_attestor(attestor.0),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn unregister_invulnerable_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let (attestor, public_key, signature) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.clone(),
            public_key,
            signature
        ));

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            public_key
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
        assert_err!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), ATTESTOR_1),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}
#[test]
fn unregister_invulnerable_should_fail_when_address_is_not_invulnerable() {
    ExtBuilder.build_and_execute(|| {
        let (attestor, public_key, signature) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.clone(),
            public_key,
            signature
        ));
        assert_err!(
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
        let attestor = attestor(ATTESTOR_1);

        assert_err!(
            Attestation::set_comittee_set_size(attestor.0, 512),
            BadOrigin
        );
    })
}

#[test]
fn add_invulnerable_also_adds_as_attestor_works() {
    ExtBuilder.build_and_execute(|| {
        let (_, public_key, _) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            public_key
        ));

        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(Attestation::invulnerables(ATTESTOR_1))
    })
}

// Rare case that an invulnerable signals unregister and then sudo removes that one as invulnerable
#[test]
fn remove_invulnerable_that_is_not_attestor_works() {
    ExtBuilder.build_and_execute(|| {
        let (_, public_key, _) = attestor(ATTESTOR_1);
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            public_key
        ));

        // Unregister
        let attestor = attestor(ATTESTOR_1);
        assert_ok!(Attestation::unregister_attestor(attestor.0));

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
fn adding_a_supported_chain_works() {
    ExtBuilder.build_and_execute(|| {
        let supported_chains = Attestation::supported_chains();
        assert_eq!(supported_chains.len(), 1);

        let chain_id = 2;
        let chain_attestation_interval = 10;
        assert_ok!(Attestation::add_supported_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_attestation_interval
        ));

        let supported_chains = Attestation::supported_chains();
        assert_eq!(supported_chains.len(), 2);
    })
}

#[test]
fn submitting_attestation_works() {
    ExtBuilder.build_and_execute(|| {
        let attestation = SignedAttestation {
            attestation: AttestationPrimitive {
                chain_id: 1,
                header_number: 1,
                header_hash: Default::default(),
                tx_root: [0; 32],
                rx_root: [0; 32],
                prev_digest: None,
            },
            signature: [0; 96],
            attestors: vec![ATTESTOR_1],
        };

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation
        ));
    })
}

#[test]
fn submitting_duplicate_attestation_fails() {
    ExtBuilder.build_and_execute(|| {
        let attestation = SignedAttestation {
            attestation: AttestationPrimitive {
                chain_id: 1,
                header_number: 1,
                header_hash: Default::default(),
                tx_root: [0; 32],
                rx_root: [0; 32],
                prev_digest: None,
            },
            signature: [0; 96],
            attestors: vec![ATTESTOR_1],
        };

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        assert_err!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::AttestationExists
        );
    })
}

#[test]
fn submitting_attestation_chain_works() {
    ExtBuilder.build_and_execute(|| {
        let attestation = SignedAttestation {
            attestation: AttestationPrimitive {
                chain_id: 1,
                header_number: 1,
                header_hash: Default::default(),
                tx_root: [0; 32],
                rx_root: [0; 32],
                prev_digest: None,
            },
            signature: [0; 96],
            attestors: vec![ATTESTOR_1],
        };

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        let digest = attestation.digest();

        // create new attestation with the same digest
        let attestation = SignedAttestation {
            attestation: AttestationPrimitive {
                chain_id: 1,
                // interval is 10
                header_number: 11,
                header_hash: Default::default(),
                tx_root: [0; 32],
                rx_root: [0; 32],
                prev_digest: Some(digest),
            },
            signature: [0; 96],
            attestors: vec![ATTESTOR_1],
        };

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation
        ));
    })
}

#[test]
fn submitting_invalid_attestation_chain_fails() {
    ExtBuilder.build_and_execute(|| {
        let attestation = SignedAttestation {
            attestation: AttestationPrimitive {
                chain_id: 1,
                header_number: 1,
                header_hash: Default::default(),
                tx_root: [0; 32],
                rx_root: [0; 32],
                prev_digest: None,
            },
            signature: [0; 96],
            attestors: vec![ATTESTOR_1],
        };

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        let fake_digest = H256::random();

        // create new attestation with the same digest
        let attestation = SignedAttestation {
            attestation: AttestationPrimitive {
                chain_id: 1,
                // interval is 10
                header_number: 11,
                header_hash: Default::default(),
                tx_root: [0; 32],
                rx_root: [0; 32],
                prev_digest: Some(fake_digest),
            },
            signature: [0; 96],
            attestors: vec![ATTESTOR_1],
        };

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation
        ));
    })
}

fn invalid_proof_of_possession() {
    ExtBuilder.build_and_execute(|| {
        let (attestor, public_key, _) = attestor(ATTESTOR_1);
        let (_, _, invalid_bls_signature) = crate::tests::attestor(ATTESTOR_2);
        assert_err!(
            Attestation::register_attestor(attestor, public_key, invalid_bls_signature),
            Error::<Test>::InvalidProofOfPossession
        );
    })
}
