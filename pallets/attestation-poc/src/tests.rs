use super::*;
use crate::mock::{
    Attestation, ExtBuilder, RuntimeOrigin, Test, ATTESTOR_1, ATTESTOR_2, DEFAULT_COMITTEE_SET_SIZE,
};
use assert_matches::assert_matches;
use attestor_primitives::{Attestation as AttestationPrimitive, SignedAttestation};
use frame_support::{assert_err, assert_ok};
use sp_core::H256;
use sp_runtime::traits::BadOrigin;

fn attestor_1() -> RuntimeOrigin {
    RuntimeOrigin::signed(ATTESTOR_1)
}

fn attestor_2() -> RuntimeOrigin {
    RuntimeOrigin::signed(ATTESTOR_2)
}

// taken from account ba87dcf396d413b2d97ce38a3b7deedbb9373f6ca2147efa90e4f58cbb81e068 in node/src/chain_spec.rs file
pub const VALID_BLS_PUBLIC_KEY: [u8; 48] = [
    170, 58, 137, 206, 248, 203, 230, 122, 86, 91, 224, 157, 175, 64, 32, 188, 138, 74, 235, 23,
    175, 30, 251, 224, 58, 113, 135, 123, 20, 237, 220, 195, 1, 3, 69, 88, 3, 23, 227, 4, 240, 208,
    170, 249, 160, 113, 43, 9,
];

pub const VALID_BLS_PUBLIC_SIGNATURE: [u8; 96] = [
    170, 58, 137, 206, 248, 203, 230, 122, 86, 91, 224, 157, 175, 64, 32, 188, 138, 74, 235, 23,
    175, 30, 251, 224, 58, 113, 135, 123, 20, 237, 220, 195, 1, 3, 69, 88, 3, 17, 227, 4, 240, 208,
    170, 249, 160, 113, 43, 9, 170, 58, 139, 206, 248, 203, 236, 122, 86, 91, 220, 157, 175, 64,
    32, 188, 138, 74, 235, 27, 175, 30, 251, 224, 58, 113, 137, 123, 20, 237, 220, 195, 1, 3, 69,
    88, 3, 17, 227, 4, 240, 210, 170, 249, 160, 113, 43, 9,
];

#[test]
fn register_attestor_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(ATTESTOR_1),
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
        ));
    })
}

#[test]
fn register_attestor_should_fail_when_address_is_already_registered() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(ATTESTOR_1,),
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
        ));

        assert_err!(
            Attestation::register_attestor(
                RuntimeOrigin::signed(ATTESTOR_1),
                VALID_BLS_PUBLIC_KEY,
                VALID_BLS_PUBLIC_SIGNATURE
            ),
            Error::<Test>::AlreadyAttestor
        );
    })
}

#[test]
fn register_attestor_should_fail_when_list_is_full() {
    ExtBuilder.build_and_execute(|| {
        let root = RuntimeOrigin::root();
        let attestor_1 = attestor_1();
        let attestor_2 = attestor_2();

        assert_ok!(Attestation::set_max_attestors(root, 2));
        assert_ok!(Attestation::register_attestor(
            attestor_1,
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
        ));
        assert_err!(
            Attestation::register_attestor(
                attestor_2,
                VALID_BLS_PUBLIC_KEY,
                VALID_BLS_PUBLIC_SIGNATURE
            ),
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
        let bad_origin = attestor_1();
        assert_err!(Attestation::set_max_attestors(bad_origin, 1), BadOrigin)
    })
}

#[test]
fn set_max_invulnerables_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = attestor_1();
        assert_err!(
            Attestation::set_max_invulnerables(bad_origin, 200),
            BadOrigin
        )
    })
}

#[test]
fn set_max_attestors_should_error_if_list_is_truncated() {
    ExtBuilder.build_and_execute(|| {
        let attestor_1 = attestor_1();
        let attestor_2 = attestor_2();
        assert_ok!(Attestation::register_attestor(
            attestor_1,
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
        ));
        assert_ok!(Attestation::register_attestor(
            attestor_2,
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
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
        let attestor = attestor_1();
        assert_ok!(Attestation::register_attestor(
            attestor.clone(),
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
        ));
        assert_ok!(Attestation::unregister_attestor(attestor));
    })
}

#[test]
fn unregister_attestor_should_fail_when_address_is_not_registered() {
    ExtBuilder.build_and_execute(|| {
        let attestor = attestor_1();
        assert_err!(
            Attestation::unregister_attestor(attestor),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn unregister_invulnerable_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        let attestor = attestor_1();
        assert_ok!(Attestation::register_attestor(
            attestor.clone(),
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
        ));

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            VALID_BLS_PUBLIC_KEY
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
        let attestor = attestor_1();
        assert_ok!(Attestation::register_attestor(
            attestor.clone(),
            VALID_BLS_PUBLIC_KEY,
            VALID_BLS_PUBLIC_SIGNATURE
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
        let attestor = attestor_1();

        assert_err!(Attestation::set_comittee_set_size(attestor, 512), BadOrigin);
    })
}

#[test]
fn add_invulnerable_also_adds_as_attestor_works() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            VALID_BLS_PUBLIC_KEY
        ));

        assert!(Attestation::attestors(ATTESTOR_1).is_some());
        assert!(Attestation::invulnerables(ATTESTOR_1))
    })
}

// Rare case that an invulnerable signals unregister and then sudo removes that one as invulnerable
#[test]
fn remove_invulnerable_that_is_not_attestor_works() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            ATTESTOR_1,
            VALID_BLS_PUBLIC_KEY
        ));

        // Unregister
        let attestor = attestor_1();
        assert_ok!(Attestation::unregister_attestor(attestor));

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
