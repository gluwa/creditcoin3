use super::*;
use crate::mock::{
    Attestation, ExtBuilder, RuntimeOrigin, Test, ATTESTOR_1, ATTESTOR_2, DEFAULT_COMITTEE_SET_SIZE,
};
use assert_matches::assert_matches;
use attestor_primitives::{Attestation as AttestationPrimitive, SignedAttestation};
use attestor_primitives::{BlsPublicKey, BlsSignature};
use bls_signatures::key::Serialize;
use bls_signatures::PrivateKey;
use frame_support::{assert_err, assert_ok};
use sp_core::H256;
use sp_runtime::traits::BadOrigin;

pub struct Attestor {
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

        let attestor = RuntimeOrigin::signed(attestor);

        Self {
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

        assert_err!(
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
        assert_err!(
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
        assert_err!(Attestation::set_max_attestors(bad_origin, 1), BadOrigin)
    })
}

#[test]
fn set_max_invulnerables_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_err!(
            Attestation::set_max_invulnerables(bad_origin, 200),
            BadOrigin
        )
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
        assert_err!(
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
        assert_err!(
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
        assert_err!(
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
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);

        assert_err!(Attestation::set_comittee_set_size(attestor, 512), BadOrigin);
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

        assert_err!(
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
        assert_err!(
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
