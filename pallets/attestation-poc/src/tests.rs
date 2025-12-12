use super::*;
use crate::impls::ONE_TENTH_CTC;
use crate::mock::*;
use attestor_primitives::attestation_fragment::{
    AttestationFragment, AttestationFragmentSerializable,
};
use attestor_primitives::{
    block::Block, AttestationCheckpoint, AttestationData as AttestationPrimitive, AttestorStatus,
    ChainKey, SignedAttestation,
};
use attestor_primitives::{BlsPublicKey, BlsSignature, Digest};
use bls_signatures::{aggregate, key::Serialize, PrivateKey, PublicKey};
use continuity_dev::construct_fragment;
use frame_support::{assert_err, assert_noop, assert_ok};
use sp_core::{Get, H256};
use sp_io::TestExternalities;
use sp_runtime::traits::BadOrigin;
use sp_std::ops::RangeInclusive;

#[derive(Debug, Clone)]
pub struct Attestor {
    pub stash: RuntimeOrigin,
    pub stash_id: AccountId,
    pub attestor_id: AccountId,
    #[allow(dead_code)]
    pub attestor_origin: RuntimeOrigin,
    private_key: PrivateKey,
    pub public_key: BlsPublicKey,
    pub signature: BlsSignature,
}

impl Attestor {
    pub fn new(stash_id: u64, attestor_id: u64) -> Self {
        let rng = sp_core::H256::random().0;
        let private_key = PrivateKey::new(rng);
        let public_key = private_key.public_key().as_bytes()[..].try_into().unwrap();
        let signature = private_key.sign(public_key).as_bytes()[..]
            .try_into()
            .unwrap();

        let stash = RuntimeOrigin::signed(stash_id);

        Self {
            stash,
            stash_id,
            attestor_id,
            attestor_origin: RuntimeOrigin::signed(attestor_id),
            private_key,
            public_key,
            signature,
        }
    }

    pub fn sign(&self, message: &[u8]) -> BlsSignature {
        self.private_key.sign(message).as_bytes()[..]
            .try_into()
            .unwrap()
    }

    pub fn private_key(&self) -> PrivateKey {
        self.private_key
    }
}

pub fn create_signed_attestation(
    attestors: Vec<Attestor>,
    chain_key: ChainKey,
    header_number: u64,
    prev_digest: Option<H256>,
    fragment: Option<AttestationFragment>,
) -> SignedAttestation<H256, mock::AccountId> {
    let fragment = if let Some(f) = fragment {
        f
    } else {
        construct_fragment(
            prev_digest,
            RangeInclusive::new(1, header_number.saturating_sub(1)),
        )
    };

    let attestation = AttestationPrimitive {
        chain_key,
        header_number,
        header_hash: H256::random(),
        root: H256::from([0; 32]),
        prev_digest: fragment.head().map(|h| {
            let block: Block = h.clone();
            block.digest()
        }),
    };

    self::bls_sign_attestation(attestors, attestation, &fragment)
}

pub fn bls_sign_attestation(
    attestors: Vec<Attestor>,
    attestation: AttestationPrimitive<H256>,
    fragment: &AttestationFragment,
) -> SignedAttestation<H256, mock::AccountId> {
    let mut signatures = Vec::new();
    for attestor in attestors.iter() {
        let signature = attestor.sign(&attestation.serialize());
        let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
            .expect("Failed to create signature");

        signatures.push(bls_sig);
    }
    // sign
    let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

    let continuity_proof = AttestationFragmentSerializable::from(fragment);
    let attestation = SignedAttestation {
        attestation,
        signature: aggregated_signature.as_bytes()[..]
            .try_into()
            .expect("Failed to convert to array"),
        attestors: attestors.iter().map(|a| a.attestor_id).collect::<Vec<_>>(),
        continuity_proof,
    };

    attestation
}

pub fn create_checkpoint(
    attestaion_interval: u64,
    mut last_digest: Option<H256>,
    attestors: Vec<Attestor>,
) -> (
    Vec<SignedAttestation<H256, mock::AccountId>>,
    Option<Digest>,
) {
    let attestations = Vec::new();
    for i in 0..attestaion_interval {
        let attestation_header_number = attestaion_interval * i;
        let fragment_start = attestation_header_number.saturating_sub(attestaion_interval) + 1;
        let fragment = construct_fragment(
            last_digest,
            RangeInclusive::new(fragment_start, attestation_header_number.saturating_sub(1)),
        );

        let attestation = create_signed_attestation(
            attestors.clone(),
            SUPPORTED_CHAIN_KEY,
            attestation_header_number,
            last_digest,
            Some(fragment),
        );
        last_digest = Some(attestation.digest());

        assert_ok!(Attestation::commit_attestation(
            attestors[0].stash.clone(),
            attestation.clone()
        ));
    }
    (attestations, last_digest)
}

#[test]
fn set_min_bond_requirement_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_min_bond_requirement(RuntimeOrigin::none(), SUPPORTED_CHAIN_KEY, 200),
            BadOrigin
        );
    })
}

#[test]
fn set_min_bond_requirement_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_min_bond_requirement(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                200
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_min_bond_requirement_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let min_bond_requirement = Attestation::min_bond_requirement(SUPPORTED_CHAIN_KEY);
        assert_eq!(min_bond_requirement, 100_000_000_000_000_000_000);

        assert_ok!(Attestation::set_min_bond_requirement(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            200
        ));

        let min_bond_requirement = Attestation::min_bond_requirement(SUPPORTED_CHAIN_KEY);
        assert_eq!(min_bond_requirement, 200);

        System::assert_last_event(
            crate::Event::MinBondRequirementUpdated(SUPPORTED_CHAIN_KEY, 200).into(),
        );
    })
}

#[test]
fn set_max_attestors_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let max_attestors = 200;

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            max_attestors
        ));

        let from_storage = Attestation::max_attestors(SUPPORTED_CHAIN_KEY);
        assert_eq!(from_storage, max_attestors);

        System::assert_last_event(
            crate::Event::MaxAttestorsChanged(SUPPORTED_CHAIN_KEY, max_attestors).into(),
        );
    })
}

#[test]
fn chill_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::chill(RuntimeOrigin::none(), SUPPORTED_CHAIN_KEY, ATTESTOR_1),
            BadOrigin
        );
    })
}

#[test]
fn chill_should_error_when_not_signed_by_an_attestor() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::chill(
                RuntimeOrigin::signed(STASH_1),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            Error::<Test>::AddressNotAttestor,
        );
    })
}

#[test]
fn chill_should_update_status_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        // setup - register attestor
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(STASH_1),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        // act
        assert_ok!(Attestation::chill(
            RuntimeOrigin::signed(STASH_1),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        // assert
        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Idle);

        System::assert_last_event(
            crate::Event::AttestorChilled(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn register_attestor_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));
        System::assert_last_event(
            crate::Event::AttestorRegistered(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn register_attestor_should_create_ledger_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        assert_eq!(attestor.bls_public_key, None);
        assert_eq!(attestor.status, AttestorStatus::Idle);

        let min_bond_requirement = MinBondRequirement::<Test>::get(SUPPORTED_CHAIN_KEY);

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        System::assert_last_event(
            crate::Event::AttestorRegistered(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn register_attestor_without_sufficient_funds_should_fail() {
    ExtBuilder.build_and_execute(|| {
        // Set min bond
        assert_ok!(Attestation::set_min_bond_requirement(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            10_000_000_000_000_000_000_000_000
        ));

        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_noop!(
            Attestation::register_attestor(att.stash, SUPPORTED_CHAIN_KEY, att.attestor_id),
            Error::<Test>::InsufficientBalance
        );

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, 0);
    })
}

#[test]
fn register_attestor_without_sufficient_funds_should_fail_2() {
    ExtBuilder.build_and_execute(|| {
        let free_balance = Attestation::get_free_balance(&STASH_3);
        // 1_000_000_000_000_000_000_000 balance - 500 existential deposit
        assert_eq!(free_balance, 999_999_999_999_999_999_500);

        // Set min bond
        // Balance of Stash 3 is 1_000_000_000_000_000_000_000
        assert_ok!(Attestation::set_min_bond_requirement(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            600_000_000_000_000_000_000
        ));

        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let free_balance = Attestation::get_free_balance(&STASH_3);
        assert_eq!(
            free_balance + ONE_TENTH_CTC as u128,
            399_999_999_999_999_999_500
        );

        // We should not be able to register another attestor because we don't have enough funds
        let att = Attestor::new(STASH_3, ATTESTOR_2);
        assert_noop!(
            Attestation::register_attestor(att.stash, SUPPORTED_CHAIN_KEY, att.attestor_id),
            Error::<Test>::InsufficientBalance
        );
    })
}

#[test]
fn registering_multiple_attestor_increases_locked_balance() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let min_bond_requirement = MinBondRequirement::<Test>::get(SUPPORTED_CHAIN_KEY);

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement);

        // We should not be able to register another attestor because we don't have enough funds
        let att = Attestor::new(STASH_3, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 2);

        let att = Attestor::new(STASH_3, ATTESTOR_3);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 3);
    })
}

#[test]
fn registering_dergegistering_multiple_attestor_increases_decreases_locked_balance() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let min_bond_requirement = MinBondRequirement::<Test>::get(SUPPORTED_CHAIN_KEY);

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement);

        // We should not be able to register another attestor because we don't have enough funds
        let att = Attestor::new(STASH_3, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 2);

        // deregister the second attestor
        assert_ok!(Attestation::unregister_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        // Still locked
        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 2);

        // Proceed to block 50
        progress_to_block(50);

        // withdraw unbonded
        assert_ok!(Attestation::withdraw_unbonded(att.stash));

        // get locked balance
        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement);
    })
}

#[test]
fn attestor_should_be_able_to_toggle_status() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            SUPPORTED_CHAIN_KEY,
            att.public_key,
            att.signature
        ));
        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        // Public key should be set
        assert_eq!(attestor.bls_public_key, Some(att.public_key));
        assert_eq!(attestor.status, AttestorStatus::Waiting);

        // Chill
        assert_ok!(Attestation::chill(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));
        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Idle);
    })
}

#[test]
fn attestor_should_be_elected_after_5_blocks_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        assert_eq!(
            Attestation::chain_election_policy(SUPPORTED_CHAIN_KEY),
            AttestorElectionPolicy::OpenToAny
        );

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            SUPPORTED_CHAIN_KEY,
            att.public_key,
            att.signature
        ));

        progress_to_block(5);

        assert!(Attestation::is_attestor(SUPPORTED_CHAIN_KEY, &ATTESTOR_1));

        // Get events in reverse order
        let all_events = <frame_system::Pallet<Test>>::events();
        let attestors_elected_event = all_events
            .iter()
            .filter_map(|event| {
                if let RuntimeEvent::Attestation(event) = &event.event {
                    Some(event)
                } else {
                    None
                }
            })
            .next();
        assert_eq!(
            attestors_elected_event,
            Some(&Event::<Test>::AttestorsElected {
                epoch: 1,
                chain_key: 1,
                attestors: vec![ATTESTOR_1]
            })
        );
    })
}

#[test]
fn attestor_authorized_should_be_elected_after_5_blocks_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        assert_eq!(
            Attestation::chain_election_policy(SUPPORTED_CHAIN_KEY),
            AttestorElectionPolicy::OpenToAny
        );

        // Set the election policy to AuthorizedOnly
        assert_ok!(Attestation::set_election_policy(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            AttestorElectionPolicy::AuthorizedOnly
        ));

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            SUPPORTED_CHAIN_KEY,
            att.public_key,
            att.signature
        ));

        // Authorize the attestor
        assert_ok!(Attestation::authorize_attestor(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        progress_to_block(5);

        assert!(Attestation::is_attestor(SUPPORTED_CHAIN_KEY, &ATTESTOR_1));

        // Get events in reverse order
        let all_events = <frame_system::Pallet<Test>>::events();
        let attestors_elected_event = all_events
            .iter()
            .filter_map(|event| {
                if let RuntimeEvent::Attestation(event) = &event.event {
                    Some(event)
                } else {
                    None
                }
            })
            .next();
        assert_eq!(
            attestors_elected_event,
            Some(&Event::<Test>::AttestorsElected {
                epoch: 1,
                chain_key: 1,
                attestors: vec![ATTESTOR_1]
            })
        );
    })
}

#[test]
fn attestor_should_not_be_elected_after_5_blocks_if_not_signaling_start() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        progress_to_block(5);

        assert!(!Attestation::is_attestor(SUPPORTED_CHAIN_KEY, &ATTESTOR_1));
    })
}

#[test]
fn attestor_should_not_be_elected_after_5_blocks_if_not_authorized() {
    ExtBuilder.build_and_execute(|| {
        // We set the election policy to AuthorizedOnly to ensure that attestors are not elected
        assert_ok!(Attestation::set_election_policy(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            AttestorElectionPolicy::AuthorizedOnly
        ));

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            SUPPORTED_CHAIN_KEY,
            att.public_key,
            att.signature
        ));

        progress_to_block(5);

        // The attestor should not be elected because they are not authorized
        assert!(!Attestation::is_attestor(SUPPORTED_CHAIN_KEY, &ATTESTOR_1));
    })
}

#[test]
fn attestor_authorized_should_not_be_elected_after_5_blocks_for_deny_policy() {
    ExtBuilder.build_and_execute(|| {
        assert_eq!(
            Attestation::chain_election_policy(SUPPORTED_CHAIN_KEY),
            AttestorElectionPolicy::OpenToAny
        );

        // Set the election policy to AuthorizedOnly
        assert_ok!(Attestation::set_election_policy(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            AttestorElectionPolicy::DeniedToAll
        ));

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            SUPPORTED_CHAIN_KEY,
            att.public_key,
            att.signature
        ));

        // Authorize the attestor
        assert_ok!(Attestation::authorize_attestor(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        progress_to_block(5);

        // The attestor should not be elected because the policy is DenyAll
        // even if it is authorized
        assert!(!Attestation::is_attestor(SUPPORTED_CHAIN_KEY, &ATTESTOR_1));
    })
}

#[test]
fn stash_ledger_schould_increase_when_registering_multiple_attestors() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let min_bond_requirement = MinBondRequirement::<Test>::get(SUPPORTED_CHAIN_KEY);

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        assert_eq!(ledger.total_staked, min_bond_requirement);

        let att = Attestor::new(STASH_1, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        assert_eq!(ledger.total_staked, min_bond_requirement * 2);

        let locks = Balances::locks(&STASH_1);
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].amount, min_bond_requirement * 2);
    })
}

#[test]
fn register_attestor_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_noop!(
            Attestation::register_attestor(
                RuntimeOrigin::none(),
                SUPPORTED_CHAIN_KEY,
                att.attestor_id,
            ),
            BadOrigin
        );
    })
}

#[test]
fn register_attestor_should_error_when_chain_is_not_supported() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_noop!(
            Attestation::register_attestor(
                att.stash.clone(),
                0, // Not a supported chain
                att.attestor_id,
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn register_attestor_should_error_when_address_is_already_registered() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        assert_noop!(
            Attestation::register_attestor(att.stash.clone(), SUPPORTED_CHAIN_KEY, att.attestor_id,),
            Error::<Test>::AlreadyAttestor
        );
    })
}

#[test]
fn register_attestor_should_error_when_list_is_full() {
    ExtBuilder.build_and_execute(|| {
        let root = RuntimeOrigin::root();
        let att_1 = Attestor::new(STASH_1, ATTESTOR_1);
        let att_2 = Attestor::new(STASH_2, ATTESTOR_2);
        assert_ok!(Attestation::set_max_attestors(root, SUPPORTED_CHAIN_KEY, 1));
        assert_ok!(Attestation::register_attestor(
            att_1.stash,
            SUPPORTED_CHAIN_KEY,
            att_1.attestor_id,
        ));

        // note: test target is try_insert_attestor_and_emit_event()
        assert_noop!(
            Attestation::register_attestor(att_2.stash, SUPPORTED_CHAIN_KEY, att_2.attestor_id,),
            Error::<Test>::AttestorListFull
        );
    })
}

#[test]
fn attest_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::none(),
                SUPPORTED_CHAIN_KEY,
                *b"000000000000000000000000000000000000000000000000",
                *b"000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            ),
            BadOrigin
        );
    })
}

#[test]
fn attest_should_error_when_signer_not_registered_as_attestor() {
    ExtBuilder.build_and_execute(|| {
        let att1 = Attestor::new(STASH_1, ATTESTOR_1);

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att1.attestor_id),
                SUPPORTED_CHAIN_KEY,
                att1.public_key,
                att1.signature
            ),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn attest_should_error_when_public_key_is_invalid() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att.attestor_id),
                SUPPORTED_CHAIN_KEY,
                *b"000000000000000000000000000000000000000000000000",
                att.signature
            ),
            Error::<Test>::InvalidBlsPublicKey
        );
    })
}

#[test]
fn attest_should_error_when_signature_is_invalid() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);


        assert_ok!(Attestation::register_attestor(att.stash, SUPPORTED_CHAIN_KEY, att.attestor_id,));

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att.attestor_id),
                SUPPORTED_CHAIN_KEY,
                att.public_key,
                *b"000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
            ),
            Error::<Test>::InvalidBlsSignature
        );
    })
}

#[test]
fn attest_should_error_when_signature_doesnt_validate_against_public_key() {
    ExtBuilder.build_and_execute(|| {
        let att1 = Attestor::new(STASH_1, ATTESTOR_1);
        let att2 = Attestor::new(STASH_2, ATTESTOR_2);

        assert_ok!(Attestation::register_attestor(
            att1.stash,
            SUPPORTED_CHAIN_KEY,
            att1.attestor_id,
        ));

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att1.attestor_id),
                SUPPORTED_CHAIN_KEY,
                att1.public_key,
                att2.signature
            ),
            Error::<Test>::InvalidProofOfPossession
        );
    })
}

#[test]
fn attest_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let att1 = Attestor::new(STASH_1, ATTESTOR_1);

        // setup
        assert_ok!(Attestation::register_attestor(
            att1.stash,
            SUPPORTED_CHAIN_KEY,
            att1.attestor_id,
        ));
        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // act
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att1.attestor_id),
            SUPPORTED_CHAIN_KEY,
            att1.public_key,
            att1.signature
        ),);

        // assert
        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Waiting);
        assert_eq!(attestor.bls_public_key, Some(att1.public_key));

        System::assert_last_event(
            crate::Event::AttestorActivated(SUPPORTED_CHAIN_KEY, att1.attestor_id, att1.public_key)
                .into(),
        );
    })
}

// TODO: make this smarter and rely on the runtime value instead of the function
#[test]
fn max_attestor_default_should_be_100() {
    ExtBuilder
        .build_and_execute(|| assert_eq!(Attestation::max_attestors(SUPPORTED_CHAIN_KEY,), 100))
}

#[test]
fn max_invulnerable_default_should_be_100() {
    ExtBuilder
        .build_and_execute(|| assert_eq!(Attestation::max_invulnerables(SUPPORTED_CHAIN_KEY,), 100))
}

#[test]
fn set_max_invulnerables_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_max_invulnerables(RuntimeOrigin::none(), SUPPORTED_CHAIN_KEY, 200),
            BadOrigin
        );
    })
}

#[test]
fn set_max_invulnerables_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::set_max_invulnerables(bad_origin, SUPPORTED_CHAIN_KEY, 200),
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
            Attestation::set_max_invulnerables(root_origin, SUPPORTED_CHAIN_KEY, 0),
            Error::<Test>::MaxInvulnerablesCannotBeChanged
        );
    })
}

#[test]
fn set_max_invulnerables_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        assert_eq!(Attestation::max_invulnerables(SUPPORTED_CHAIN_KEY,), 100);
        let count = Invulnerables::<Test>::iter_prefix_values(SUPPORTED_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 1); // from mock

        assert_ok!(Attestation::set_max_invulnerables(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            10
        ),);
        assert_eq!(Attestation::max_invulnerables(SUPPORTED_CHAIN_KEY,), 10);
        let count = Invulnerables::<Test>::iter_prefix_values(SUPPORTED_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 1); // from mock
    })
}

#[test]
fn set_max_attestors_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_max_attestors(RuntimeOrigin::none(), SUPPORTED_CHAIN_KEY, 1),
            BadOrigin
        );
    })
}

#[test]
fn set_max_attestors_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::set_max_attestors(bad_origin, SUPPORTED_CHAIN_KEY, 1),
            BadOrigin
        );
    })
}

#[test]
fn set_max_attestors_should_work_when_truncating_existing_list() {
    ExtBuilder.build_and_execute(|| {
        let att_1 = Attestor::new(STASH_1, ATTESTOR_1);
        let att_2 = Attestor::new(STASH_2, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att_1.stash,
            SUPPORTED_CHAIN_KEY,
            att_1.attestor_id,
        ));
        assert_ok!(Attestation::register_attestor(
            att_2.stash,
            SUPPORTED_CHAIN_KEY,
            att_2.attestor_id,
        ));

        let count = Attestors::<Test>::iter_prefix_values(SUPPORTED_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 2);

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            1
        ));
        let count = Attestors::<Test>::iter_prefix_values(SUPPORTED_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 2);
        let max_attestors = Attestation::max_attestors(SUPPORTED_CHAIN_KEY);
        assert_eq!(max_attestors, 1);
    })
}

#[test]
fn set_max_attestors_should_work_when_list_is_empty() {
    ExtBuilder.build_and_execute(|| {
        let _ = Attestors::<Test>::clear(u32::MAX, None);
        let count = Attestors::<Test>::iter_prefix_values(SUPPORTED_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 0);

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            5
        ));
        let max_attestors = Attestation::max_attestors(SUPPORTED_CHAIN_KEY);
        assert_eq!(max_attestors, 5);
    })
}

#[test]
fn set_max_attestors_should_work_when_expanding_existing_list() {
    ExtBuilder.build_and_execute(|| {
        let att_1 = Attestor::new(STASH_1, ATTESTOR_1);
        let att_2 = Attestor::new(STASH_2, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att_1.stash,
            SUPPORTED_CHAIN_KEY,
            att_1.attestor_id,
        ));
        assert_ok!(Attestation::register_attestor(
            att_2.stash,
            SUPPORTED_CHAIN_KEY,
            att_2.attestor_id,
        ));

        let count = Attestors::<Test>::iter_prefix_values(SUPPORTED_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 2);
        // this is the default value
        let max_attestors = Attestation::max_attestors(SUPPORTED_CHAIN_KEY);
        assert_eq!(max_attestors, 100);

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            10
        ),);
        let max_attestors = Attestation::max_attestors(SUPPORTED_CHAIN_KEY);
        assert_eq!(max_attestors, 10);
    })
}

#[test]
fn unregister_attestor_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_attestor(
                RuntimeOrigin::none(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            BadOrigin
        );
    })
}

#[test]
fn unregister_attestor_should_error_when_address_is_not_registered_as_attestor() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::unregister_attestor(attestor, SUPPORTED_CHAIN_KEY, ATTESTOR_1),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn unregister_attestor_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        // setup
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));
        assert!(Attestation::attestor_is_registered(
            SUPPORTED_CHAIN_KEY,
            &ATTESTOR_1
        ));

        // test
        assert_ok!(Attestation::unregister_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));
        let attestor = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1);
        assert!(attestor.is_none());
        System::assert_last_event(
            crate::Event::AttestorUnregistered(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn unregister_invulnerable_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        // setup
        assert!(!Invulnerables::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));
        assert!(Attestation::invulnerables(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());

        // test
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));
        assert!(!Invulnerables::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));
        System::assert_last_event(
            crate::Event::InvulnerableUnregistered(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
        )
    })
}

#[test]
fn unregister_invulnerable_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_invulnerable(
                RuntimeOrigin::none(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            BadOrigin
        );
    })
}

#[test]
fn unregister_invulnerable_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_invulnerable(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            BadOrigin
        );
    })
}

#[test]
fn unregister_invulnerable_should_fail_when_address_is_not_registered_at_all() {
    ExtBuilder.build_and_execute(|| {
        assert!(!Attestors::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));
        assert!(!Invulnerables::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_noop!(
            Attestation::unregister_invulnerable(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}

#[test]
fn unregister_invulnerable_should_fail_when_address_is_an_attestor_but_not_invulnerable() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            att.stash,
            SUPPORTED_CHAIN_KEY,
            att.attestor_id,
        ));
        assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(!Invulnerables::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_noop!(
            Attestation::unregister_invulnerable(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}

#[test]
fn set_target_sample_size_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_target_sample_size(RuntimeOrigin::none(), SUPPORTED_CHAIN_KEY, 512),
            BadOrigin
        );
    })
}

#[test]
fn set_target_sample_size_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);

        assert_noop!(
            Attestation::set_target_sample_size(attestor, SUPPORTED_CHAIN_KEY, 512),
            BadOrigin
        );
    })
}

#[test]
fn set_target_sample_size_should_fail_with_set_size_less_than_1() {
    ExtBuilder.build_and_execute(|| {
        let new_committee_size = 0;
        assert_noop!(
            Attestation::set_target_sample_size(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                new_committee_size
            ),
            Error::<Test>::InvalidTargetSampleSize
        );
    })
}

#[test]
fn set_target_sample_size_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        let committee_size = Attestation::target_sample_size(SUPPORTED_CHAIN_KEY);
        assert_eq!(committee_size, TargetSampleSizeDefault::<Test>::get());

        let new_committee_size = 512;
        assert_ok!(Attestation::set_target_sample_size(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            new_committee_size
        ));

        let committee_size = Attestation::pending_target_sample_size(SUPPORTED_CHAIN_KEY);
        assert_eq!(committee_size, Some(512));

        System::assert_last_event(
            crate::Event::PendingTargetSampleSizeSet(SUPPORTED_CHAIN_KEY, new_committee_size)
                .into(),
        );
    })
}

#[test]
fn register_invulnerable_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_noop!(
            Attestation::register_invulnerable(
                RuntimeOrigin::none(),
                SUPPORTED_CHAIN_KEY,
                att.attestor_id,
            ),
            BadOrigin
        );
    })
}

#[test]
fn register_invulnerable_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_noop!(
            Attestation::register_invulnerable(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                att.attestor_id,
            ),
            BadOrigin
        );
    })
}

#[test]
fn register_invulnerable_adds_attestor_and_invulnerable_and_emits_events() {
    ExtBuilder.build_and_execute(|| {
        assert!(!Invulnerables::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1,
        ));

        assert!(Attestation::invulnerables(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());

        // assert on event
        System::assert_last_event(
            crate::Event::InvulnerableRegistered(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

// Rare case that an invulnerable signals unregister and then sudo removes that one as invulnerable
#[test]
fn remove_invulnerable_works() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1,
        ));

        // Still invulnerable
        assert!(Attestation::invulnerables(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_some());

        // Remove as invulnerable
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));
    })
}

#[test]
fn set_chain_attestation_interval_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let chain_attestation_interval = 101;

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::none(),
                SUPPORTED_CHAIN_KEY,
                chain_attestation_interval
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_attestation_interval_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let chain_attestation_interval = 101;

        let acct: AccountId = 4;

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::signed(acct),
                SUPPORTED_CHAIN_KEY,
                chain_attestation_interval
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_attestation_interval_should_error_with_interval_0() {
    ExtBuilder.build_and_execute(|| {
        let chain_attestation_interval = 0;
        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                chain_attestation_interval
            ),
            Error::<Test>::InvalidAttestationInterval
        );
    })
}

#[test]
fn set_chain_attestation_interval_should_error_for_unsupported_chain() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 2;
        let chain_attestation_interval = 101;
        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::root(),
                chain_key,
                chain_attestation_interval
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn set_chain_attestation_interval_updates_internal_storage_and_emits_event() {
    ExtBuilder.build_and_execute(|| {
        let attestation_interval = Attestation::pending_attestation_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(attestation_interval, None); // Interval set in mock genesis

        let chain_attestation_interval = 101;
        assert_ok!(Attestation::set_chain_attestation_interval(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            chain_attestation_interval
        ));

        let attestation_interval = Attestation::pending_attestation_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(attestation_interval, Some(101));

        System::assert_last_event(
            crate::Event::PendingAttestationIntervalSet(
                SUPPORTED_CHAIN_KEY,
                chain_attestation_interval,
            )
            .into(),
        );
    })
}

#[test]
fn on_new_epoch_randomness_updates_attestation_interval_with_pending_value_and_emits_event() {
    ExtBuilder.build_and_execute(|| {
        // Set up pending interval change
        assert_eq!(<Test as pallet_babe::Config>::EpochDuration::get(), 3);

        // this sets the genesis slot to 6;
        go_to_block(1, 6);

        assert_eq!(*Babe::genesis_slot(), 6);
        assert_eq!(*Babe::current_slot(), 6);
        assert_eq!(Babe::epoch_index(), 0);

        let chain_attestation_interval = 101;
        assert_ok!(Attestation::set_chain_attestation_interval(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            chain_attestation_interval
        ));

        let pending_interval = Attestation::pending_attestation_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(pending_interval, Some(101));

        // Update interval in on_initialize hook
        progress_to_block(4);

        let applied_interval = Attestation::chain_attestation_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(applied_interval, 101);

        let pending_interval = Attestation::pending_attestation_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(pending_interval, None);

        // Get events in reverse order
        let all_events = <frame_system::Pallet<Test>>::events();
        let interval_update_event = all_events
            .iter()
            .filter_map(|event| {
                if let RuntimeEvent::Attestation(event) = &event.event {
                    Some(event)
                } else {
                    None
                }
            })
            .next();
        assert_eq!(
            interval_update_event,
            Some(&Event::<Test>::AttestationIntervalChanged(1, 101))
        );
    });
}

// Test target sample size udpates properly on epoch change
#[test]
fn on_new_epoch_randomness_updates_target_sample_size_with_pending_value_and_emits_event() {
    ExtBuilder.build_and_execute(|| {
        // Set up pending target sample size change
        assert_eq!(<Test as pallet_babe::Config>::EpochDuration::get(), 3);

        // this sets the genesis slot to 6;
        go_to_block(1, 6);

        assert_eq!(*Babe::genesis_slot(), 6);
        assert_eq!(*Babe::current_slot(), 6);
        assert_eq!(Babe::epoch_index(), 0);

        let target_sample_size = 512;
        assert_ok!(Attestation::set_target_sample_size(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            target_sample_size
        ));

        let pending_target_sample_size =
            Attestation::pending_target_sample_size(SUPPORTED_CHAIN_KEY);
        assert_eq!(pending_target_sample_size, Some(512));

        // Update target sample size in on_initialize hook
        progress_to_block(4);

        let applied_target_sample_size = Attestation::target_sample_size(SUPPORTED_CHAIN_KEY);
        assert_eq!(applied_target_sample_size, 512);

        let pending_interval = Attestation::pending_attestation_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(pending_interval, None);

        // Get events in reverse order
        let all_events = <frame_system::Pallet<Test>>::events();
        let interval_update_event = all_events
            .iter()
            .filter_map(|event| {
                if let RuntimeEvent::Attestation(event) = &event.event {
                    Some(event)
                } else {
                    None
                }
            })
            .next();
        assert_eq!(
            interval_update_event,
            Some(&Event::<Test>::TargetSampleSizeChanged(1, 512))
        );
    });
}
#[test]
fn set_attestations_per_checkpoint_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        let att_per_check = Attestation::attestation_checkpoint_interval(SUPPORTED_CHAIN_KEY);
        assert_eq!(att_per_check, 10); // Checkpoint frequencty set in mock genesis

        let new_att_per_check = 101;
        assert_ok!(Attestation::set_attestations_per_checkpoint(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            new_att_per_check
        ));

        let att_per_check = Attestation::attestation_checkpoint_interval(SUPPORTED_CHAIN_KEY);
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
            Attestation::set_attestations_per_checkpoint(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                101
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_attestations_per_checkpoint_should_error_with_invalid_interval_value() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_attestations_per_checkpoint(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                0
            ),
            Error::<Test>::InvalidAttestationsPerCheckpoint
        );
    })
}

#[test]
fn set_attestations_per_checkpoint_should_error_on_unsupported_chain() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 2;
        let att_per_check = 101;
        assert_noop!(
            Attestation::set_attestations_per_checkpoint(
                RuntimeOrigin::root(),
                chain_key,
                att_per_check
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation =
            create_signed_attestation(vec![attestor], SUPPORTED_CHAIN_KEY, 1, None, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::none(), attestation,),
            BadOrigin
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation =
            create_signed_attestation(vec![attestor], SUPPORTED_CHAIN_KEY, 1, None, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::signed(ATTESTOR_1), attestation,),
            BadOrigin
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_chain_is_unsupported() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 2;
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], chain_key, 1, None, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::root(), attestation),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn bootstrap_chain_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        let attestation_for_block_10 =
            create_signed_attestation(vec![attestor], SUPPORTED_CHAIN_KEY, 10, None, None);

        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation.header_number(),
            digest: attestation.digest(),
        };

        assert_eq!(
            Attestation::last_attestation_digest(SUPPORTED_CHAIN_KEY),
            None
        );
        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation.digest()),
            None
        );
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::bootstrap_chain(
            RuntimeOrigin::root(),
            attestation.clone(),
        ),);

        // storage
        assert_eq!(
            Attestation::last_attestation_digest(SUPPORTED_CHAIN_KEY),
            Some((attestation.header_number(), attestation.digest()))
        );
        // Should be none because the first attestation was already processed and removed
        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation_for_block_10.digest()),
            None
        );
        // Shouldn't add first attestation for chain to checkpointing queue
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );

        // event
        System::assert_last_event(
            Event::CheckpointReached(SUPPORTED_CHAIN_KEY, expected_checkpoint.clone()).into(),
        );

        // assert last checkpoint
        assert_eq!(
            Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY),
            Some(expected_checkpoint.clone())
        );

        assert_ok!(Attestation::bootstrap_chain(
            RuntimeOrigin::root(),
            attestation_for_block_10.clone(),
        ),);

        // storage
        assert_eq!(
            Attestation::last_attestation_digest(SUPPORTED_CHAIN_KEY),
            Some((
                attestation_for_block_10.header_number(),
                attestation_for_block_10.digest()
            ))
        );
        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation_for_block_10.digest()),
            Some(attestation_for_block_10.clone())
        );
        // Only the second attestation should be inside the checkpointing queue because the first was already processed
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            1
        );

        // event
        System::assert_last_event(
            Event::BlockAttested(
                SUPPORTED_CHAIN_KEY,
                attestation_for_block_10.header_number(),
                attestation_for_block_10.digest(),
            )
            .into(),
        );

        // assert last checkpoint
        assert_eq!(
            Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY),
            Some(expected_checkpoint)
        );
    })
}

#[test]
fn commit_attestation_interval_10_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        log::info!("Attestation 1: {:?}", attestation_1.digest());

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        // The first attestation for a chain immediately creates a corresponding checkpoint
        // rather than adding to the checkpointing queue.
        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation_1.header_number(),
            digest: attestation_1.digest(),
        };
        assert_eq!(
            Attestation::checkpoints(SUPPORTED_CHAIN_KEY, expected_checkpoint.block_number),
            Some(expected_checkpoint.digest)
        );
        // assert last checkpoint
        assert_eq!(
            Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY),
            Some(expected_checkpoint)
        );

        // Create a second attestation since first became a checkpoint and was removed from attestations
        let attestation = create_signed_attestation(
            vec![attestor.clone()],
            SUPPORTED_CHAIN_KEY,
            10,
            Some(attestation_1.digest()),
            None,
        );

        assert_ok!(Attestation::commit_attestation(
            attestor.stash,
            attestation.clone()
        ));

        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation.digest()),
            Some(attestation)
        );
    })
}

#[test]
fn commit_attestation_interval_1_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        log::info!("Attestation 1: {:?}", attestation_1.digest());

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        // The first attestation for a chain immediately creates a corresponding checkpoint
        // rather than adding to the checkpointing queue.
        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation_1.header_number(),
            digest: attestation_1.digest(),
        };
        assert_eq!(
            Attestation::checkpoints(SUPPORTED_CHAIN_KEY, expected_checkpoint.block_number),
            Some(expected_checkpoint.digest)
        );
        // assert last checkpoint
        assert_eq!(
            Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY),
            Some(expected_checkpoint)
        );

        // Create a second attestation since first became a checkpoint and was removed from attestations
        let attestation = AttestationPrimitive {
            chain_key: SUPPORTED_CHAIN_KEY,
            header_number: 1,
            header_hash: H256::random(),
            root: H256::from([0; 32]),
            prev_digest: Some(attestation_1.digest()),
        };
        let attestation = self::bls_sign_attestation(
            vec![attestor.clone()],
            attestation,
            &AttestationFragment::default(),
        );

        assert_ok!(Attestation::commit_attestation(
            attestor.stash,
            attestation.clone()
        ));

        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation.digest()),
            Some(attestation)
        );
    })
}

#[test]
fn commit_attestation_interval_1_fails_with_wrong_prev_digest() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        log::info!("Attestation 1: {:?}", attestation_1.digest());

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        // The first attestation for a chain immediately creates a corresponding checkpoint
        // rather than adding to the checkpointing queue.
        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation_1.header_number(),
            digest: attestation_1.digest(),
        };
        assert_eq!(
            Attestation::checkpoints(SUPPORTED_CHAIN_KEY, expected_checkpoint.block_number),
            Some(expected_checkpoint.digest)
        );
        // assert last checkpoint
        assert_eq!(
            Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY),
            Some(expected_checkpoint)
        );

        // Create a second attestation since first became a checkpoint and was removed from attestations
        let attestation = AttestationPrimitive {
            chain_key: SUPPORTED_CHAIN_KEY,
            header_number: 1,
            header_hash: H256::random(),
            root: H256::from([0; 32]),
            prev_digest: Some(H256::random()), // wrong prev digest
        };
        let attestation = self::bls_sign_attestation(
            vec![attestor.clone()],
            attestation,
            &AttestationFragment::default(),
        );

        // Will error with EmptyContinuityProof because the prev digest does not match
        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation.clone()),
            Error::<Test>::EmptyContinuityProof
        );
    })
}

#[test]
fn commit_attestation_should_error_on_invalid_attestation_header() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // There is no finalized attestation yet and we are not attesting to genesis
        let attestation_1 = create_signed_attestation(
            vec![attestor.clone()],
            SUPPORTED_CHAIN_KEY,
            10,
            // Because we use a random H256 here, the continuity proof will be invalid
            Some(H256::random()),
            None,
        );

        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation_1.clone()),
            Error::<Test>::NoFinalizedAttestation
        );
    })
}

#[test]
fn commit_attestation_should_error_on_invalid_continuity_proof_tail() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // commit a good one first
        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        let attestation_2 = create_signed_attestation(
            vec![attestor.clone()],
            SUPPORTED_CHAIN_KEY,
            10,
            // Because we use a None here, the continuity proof tail will be invalid
            None,
            None,
        );

        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation_2.clone()),
            Error::<Test>::InvalidAttestationContinuityProofTail
        );
    })
}

#[test]
fn commit_attestation_should_error_on_invalid_prev_digest() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // commit a good one first
        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        let attestation = AttestationPrimitive {
            chain_key: SUPPORTED_CHAIN_KEY,
            header_number: 10,
            header_hash: H256::random(),
            root: H256::from([0; 32]),
            prev_digest: None,
        };

        let mut signatures = Vec::new();
        for attestor in vec![attestor.clone()].iter() {
            let signature = attestor.sign(&attestation.serialize());
            let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
                .expect("Failed to create signature");

            signatures.push(bls_sig);
        }
        // sign
        let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

        let fragment = construct_fragment(attestation_1.prev_digest(), RangeInclusive::new(1, 9));
        let continuity_proof = AttestationFragmentSerializable::from(&fragment);

        let attestation_2 = SignedAttestation {
            attestation,
            signature: aggregated_signature.as_bytes()[..]
                .try_into()
                .expect("Failed to convert to array"),
            attestors: vec![attestor.clone()]
                .iter()
                .map(|a| a.attestor_id)
                .collect::<Vec<_>>(),
            continuity_proof,
        };

        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation_2.clone()),
            Error::<Test>::InvalidAttestationPrevDigest
        );
    })
}

#[test]
fn commit_attestation_should_error_on_invalid_continuity_head() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // commit a good one first
        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        let attestation = AttestationPrimitive {
            chain_key: SUPPORTED_CHAIN_KEY,
            header_number: 10,
            header_hash: H256::random(),
            root: H256::from([0; 32]),
            prev_digest: Some(attestation_1.digest()),
        };

        let mut signatures = Vec::new();
        for attestor in vec![attestor.clone()].iter() {
            let signature = attestor.sign(&attestation.serialize());
            let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
                .expect("Failed to create signature");

            signatures.push(bls_sig);
        }
        // sign
        let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

        let fragment = construct_fragment(Some(H256::random()), RangeInclusive::new(1, 9));
        let continuity_proof = AttestationFragmentSerializable::from(&fragment);

        let attestation_2 = SignedAttestation {
            attestation,
            signature: aggregated_signature.as_bytes()[..]
                .try_into()
                .expect("Failed to convert to array"),
            attestors: vec![attestor.clone()]
                .iter()
                .map(|a| a.attestor_id)
                .collect::<Vec<_>>(),
            continuity_proof,
        };

        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation_2.clone()),
            Error::<Test>::InvalidAttestationContinuityProofTail
        );
    })
}

#[test]
fn commit_attestation_should_error_on_invalid_continuity_block() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // commit a good one first
        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        // Create a correct continuity proof fragment
        let correct_fragment =
            construct_fragment(Some(attestation_1.digest()), RangeInclusive::new(1, 9));
        let attestation = AttestationPrimitive {
            chain_key: SUPPORTED_CHAIN_KEY,
            header_number: 10,
            header_hash: H256::random(),
            root: H256::from([0; 32]),
            prev_digest: correct_fragment.head().map(|h| {
                let block: Block = h.clone();
                block.digest()
            }),
        };

        let mut signatures = Vec::new();
        for attestor in vec![attestor.clone()].iter() {
            let signature = attestor.sign(&attestation.serialize());
            let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
                .expect("Failed to create signature");

            signatures.push(bls_sig);
        }
        // sign
        let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

        // Create a fragment with correct tail but wrong roots in the middle
        // This will cause the final reconstructed digest to not match the attestation's prev_digest
        let correct_fragment =
            construct_fragment(Some(attestation_1.digest()), RangeInclusive::new(1, 9));
        let mut invalid_blocks = correct_fragment.blocks().to_vec();

        // Modify one block in the middle to have a wrong root
        // This will cause the reconstructed digest chain to be wrong
        if let Some(block) = invalid_blocks.get_mut(4) {
            // Change the root to a different value, which will break the digest chain
            *block = Block::new_from_prev_digest(
                block.block_number,
                H256::from([1; 32]), // Wrong root instead of zero
                block.prev_digest,
            );
        }

        let invalid_fragment = AttestationFragment::from_blocks(invalid_blocks);
        let continuity_proof = AttestationFragmentSerializable::from(&invalid_fragment);

        let attestation_2 = SignedAttestation {
            attestation,
            signature: aggregated_signature.as_bytes()[..]
                .try_into()
                .expect("Failed to convert to array"),
            attestors: vec![attestor.clone()]
                .iter()
                .map(|a| a.attestor_id)
                .collect::<Vec<_>>(),
            continuity_proof,
        };

        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation_2.clone()),
            Error::<Test>::InvalidAttestationContinuityProofHead
        );
    })
}

#[test]
fn commit_attestation_should_error_on_invalid_continuity_genesis_block() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );
        assert_eq!(Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY), None);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // // commit a good one first
        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);

        // Create a correct continuity proof fragment
        let correct_fragment = construct_fragment(Some(H256::random()), RangeInclusive::new(0, 9));
        let attestation = AttestationPrimitive {
            chain_key: SUPPORTED_CHAIN_KEY,
            header_number: 10,
            header_hash: H256::random(),
            root: H256::from([0; 32]),
            prev_digest: correct_fragment.head().map(|h| {
                let block: Block = h.clone();
                block.digest()
            }),
        };

        let mut signatures = Vec::new();
        for attestor in vec![attestor.clone()].iter() {
            let signature = attestor.sign(&attestation.serialize());
            let bls_sig = bls_signatures::Signature::from_bytes(&signature[..])
                .expect("Failed to create signature");

            signatures.push(bls_sig);
        }
        // sign
        let aggregated_signature = aggregate(&signatures).expect("Failed to aggregate signatures");

        // let fragment = construct_fragment(None, 0, 9);
        let continuity_proof = AttestationFragmentSerializable::from(&correct_fragment);

        let attestation_2 = SignedAttestation {
            attestation,
            signature: aggregated_signature.as_bytes()[..]
                .try_into()
                .expect("Failed to convert to array"),
            attestors: vec![attestor.clone()]
                .iter()
                .map(|a| a.attestor_id)
                .collect::<Vec<_>>(),
            continuity_proof,
        };

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1
        ));

        assert_err!(
            Attestation::commit_attestation(attestor.stash, attestation_2),
            Error::<Test>::InvalidAttestationContinuityProofTail
        );
    })
}

#[test]
fn commit_attestation_should_error_when_unsigned() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 1, 1, None, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            BadOrigin
        );
    })
}

#[test]
fn commit_attestation_should_error_when_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 1, 1, None, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::root(), attestation),
            BadOrigin
        );
    })
}

#[test]
fn validate_attestation_should_error_when_chain_is_not_supported() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 2;

        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], chain_key, 1, None, None);

        let result = Attestation::validate_attestation(attestation.chain_key(), &attestation);
        assert_err!(result, Error::<Test>::ChainNotSupported);
    })
}

#[test]
fn commit_attestation_should_error_when_submitting_duplicate_attestation() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation_1 = create_signed_attestation(vec![attestor.clone()], 1, 0, None, None);

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        // Should error when trying to submit the same attestation again
        let result = Attestation::validate_attestation(attestation_1.chain_key(), &attestation_1);
        assert_err!(result, Error::<Test>::AttestationExists);

        let attestation_2 = create_signed_attestation(
            vec![attestor.clone()],
            1,
            10,
            Some(attestation_1.digest()),
            None,
        );

        assert_ok!(Attestation::commit_attestation(
            attestor.stash,
            attestation_2.clone()
        ));

        // Should error when trying to submit the same attestation again
        let result = Attestation::validate_attestation(attestation_2.chain_key(), &attestation_2);
        assert_err!(result, Error::<Test>::AttestationExists);
    })
}

#[test]
fn validate_attestation_should_error_when_it_cannot_validate_the_attestation() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 1, 1, None, None);

        // note: not calling register_attestor() will cause the validation to fail
        let result = Attestation::validate_attestation(attestation.chain_key(), &attestation);
        assert_err!(result, Error::<Test>::AttestorNotActive);
    })
}

#[test]
fn validate_attestation_should_error_when_signed_by_more_attestors() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(STASH_1),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // 1 registered & active, 2 signed
        let attestation =
            create_signed_attestation(vec![attestor.clone(), attestor], 1, 1, None, None);

        let result = Attestation::validate_attestation(attestation.chain_key(), &attestation);
        assert_err!(result, Error::<Test>::DuplicateAttestor);
    })
}

#[test]
fn validate_attestation_should_error_when_majority_not_reached() {
    ExtBuilder.build_and_execute(|| {
        // default is 1, set target > 1 to trigger failure
        assert_ok!(Attestation::set_target_sample_size(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            44
        ));

        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(STASH_1),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(vec![attestor], 1, 1, None, None);

        let result = Attestation::validate_attestation(attestation.chain_key(), &attestation);
        assert_err!(result, Error::<Test>::MajorityNotReached);
    })
}

#[test]
fn submitting_attestation_chain_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            0
        );

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_1.clone()
        ));

        let digest = attestation_1.digest();

        let attestation_2 = create_signed_attestation(
            vec![attestor.clone()],
            SUPPORTED_CHAIN_KEY,
            11,
            Some(digest),
            None,
        );

        assert_ok!(Attestation::commit_attestation(
            attestor.stash.clone(),
            attestation_2.clone()
        ));

        // Only second attestation should have been added to a queue
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            1
        );
        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).back(),
            Some(&attestation_2.digest())
        );
        // Attestation_1 became a checkpoint but it is kept in until it reaches the front of AttestationRetentionQueue
        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation_1.digest()),
            Some(attestation_1)
        );
        assert_eq!(
            Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation_2.digest()),
            Some(attestation_2)
        );
    })
}

#[test]
fn test_attestation_submission_fails_if_threshold_not_met() {
    ExtBuilder.build_and_execute(|| {
        // Set target sample size to 3
        assert_ok!(Attestation::set_target_sample_size(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            3
        ));

        let attestor_1 = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor_1.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor_1.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor_1.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor_1.public_key,
            attestor_1.signature
        ));

        progress_to_block(5);

        // Should fail because we have only one attestors and the target sample size is 3 (Default value)
        let attestation =
            create_signed_attestation(vec![attestor_1.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);
        let result = Attestation::validate_attestation(SUPPORTED_CHAIN_KEY, &attestation);
        assert_err!(result, Error::<Test>::MajorityNotReached);
    })
}

#[test]
fn test_signing() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

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
        // Setup almost two full checkpoints of attestations, so that
        // the next attestation submitted triggers checkpoint creation.
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let att_interval = Attestation::chain_attestation_interval(SUPPORTED_CHAIN_KEY);
        let att_per_check = Attestation::attestation_checkpoint_interval(SUPPORTED_CHAIN_KEY);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));
        progress_to_block(5);
        let mut last_digest: Option<H256> = None;
        let mut checkpoint_attestation: Option<SignedAttestation<H256, u64>> = None;
        for i in 0..(att_per_check * 2 + 1) as usize {
            let attestation_header_number = att_interval * i as u64;
            let fragment_start = attestation_header_number.saturating_sub(att_interval) + 1;
            let fragment = construct_fragment(
                last_digest,
                RangeInclusive::new(fragment_start, attestation_header_number.saturating_sub(1)),
            );

            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                SUPPORTED_CHAIN_KEY,
                att_interval * i as u64,
                last_digest,
                Some(fragment),
            );
            last_digest = Some(attestation.digest());
            assert_ok!(Attestation::commit_attestation(
                attestor.stash.clone(),
                attestation.clone()
            ));

            match i {
                i if i == att_per_check as usize => {
                    // End of first checkpoint interval
                    checkpoint_attestation = Some(attestation);
                }
                _ => (),
            }
        }

        assert_eq!(
            Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
            att_per_check as usize
        );

        let unwrapped_att =
            checkpoint_attestation.expect("Should have been filled to Some in loop.");
        let resulting_checkpoint = AttestationCheckpoint {
            block_number: unwrapped_att.header_number(),
            digest: unwrapped_att.digest(),
        };
        System::assert_last_event(
            crate::Event::CheckpointReached(SUPPORTED_CHAIN_KEY, resulting_checkpoint.clone())
                .into(),
        );
        assert_eq!(
            Attestation::checkpoints(SUPPORTED_CHAIN_KEY, resulting_checkpoint.block_number),
            Some(resulting_checkpoint.digest)
        );
        assert_eq!(
            Attestation::last_checkpoint(SUPPORTED_CHAIN_KEY),
            Some(resulting_checkpoint)
        );
    })
}

#[test]
fn creating_checkpoint_purges_attestations_in_removal_queue() {
    ExtBuilder.build_and_execute(|| {
        let checkpoints_in_retention = 1;
        let checkpoints_to_create = checkpoints_in_retention + 2;
        // Setup state.
        // 3 checkpoints worth of attestations
        // 2 checkpoints worth recorded in checkpointing queue
        // 1 checkpoint worth recorded in removal queue
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let att_interval = Attestation::chain_attestation_interval(SUPPORTED_CHAIN_KEY);
        let att_per_check = Attestation::attestation_checkpoint_interval(SUPPORTED_CHAIN_KEY);

        // For this test we assume default values, where attestations per checkpoint == retention duration
        assert_eq!(
            att_per_check * checkpoints_in_retention,
            Attestation::attestation_retention_duration(SUPPORTED_CHAIN_KEY)
        );

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));
        progress_to_block(5);

        let mut last_digest: Option<H256> = None;
        let mut removed_by_checkpoint: Vec<H256> = Vec::new();
        let mut kept_after_checkpoint: Vec<SignedAttestation<H256, u64>> = Vec::new();

        for i in 0..att_per_check * checkpoints_to_create + 1 {
            let attestation_header_number = att_interval * i as u64;
            let fragment_start = attestation_header_number.saturating_sub(att_interval) + 1;
            let fragment = construct_fragment(
                last_digest,
                RangeInclusive::new(fragment_start, attestation_header_number.saturating_sub(1)),
            );

            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                SUPPORTED_CHAIN_KEY,
                att_interval * i as u64,
                last_digest,
                Some(fragment),
            );
            last_digest = Some(attestation.digest());

            assert_ok!(Attestation::commit_attestation(
                attestor.stash.clone(),
                attestation.clone()
            ));

            match i {
                i if i < att_per_check + 1 => {
                    // First 10 attestations added to the AttestationRemovalQueue should be purged after last checkpoint
                    // with default values.
                    removed_by_checkpoint.push(attestation.digest());
                }
                _ => {
                    // All attestations condensed in current checkpoint are kept for AttestationRetentionDuration
                    kept_after_checkpoint.push(attestation);
                }
            }
        }

        assert_eq!(
            Attestation::attestation_removal_queue(SUPPORTED_CHAIN_KEY).len(),
            (att_per_check * checkpoints_in_retention) as usize
        );

        for removed_digest in removed_by_checkpoint {
            assert_eq!(
                Attestation::attestations(SUPPORTED_CHAIN_KEY, removed_digest),
                None
            );
        }

        for kept_attestation in kept_after_checkpoint {
            assert_eq!(
                Attestation::attestations(SUPPORTED_CHAIN_KEY, kept_attestation.digest()),
                Some(kept_attestation)
            )
        }
    })
}

#[test]
fn checkpointing_rolls_back_storage_changes_if_checkpointing_queue_does_not_match_attestations_map()
{
    ExtBuilder.build_and_execute(|| {
        // Setup almost two full checkpoints of attestations, so that
        // the next attestation submitted triggers checkpoint creation.
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        let att_interval = Attestation::chain_attestation_interval(SUPPORTED_CHAIN_KEY);
        let att_per_check =
            Attestation::attestation_checkpoint_interval(SUPPORTED_CHAIN_KEY) as u64;

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let (attestations, mut last_digest) =
            create_checkpoint(att_per_check, None, vec![attestor.clone()]);

        // Inserts a garbage checkpointing queue entry without corresponding
        // attestations entry. We break checkpointing part way through,
        // requiring that all previous state changes be rolled back.
        CheckpointingQueues::<Test>::mutate(SUPPORTED_CHAIN_KEY, |queue| {
            queue.push_back([0u8; 32].into());
        });

        // Trigger checkpointing by adding one more full interval of attestations
        for i in (att_per_check)..(att_per_check * 2) {
            let attestation_header_number = att_interval * i;
            let fragment_start = attestation_header_number.saturating_sub(att_interval) + 1;
            let fragment = construct_fragment(
                last_digest,
                RangeInclusive::new(fragment_start, attestation_header_number.saturating_sub(1)),
            );

            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                SUPPORTED_CHAIN_KEY,
                attestation_header_number,
                last_digest,
                Some(fragment),
            );
            last_digest = Some(attestation.digest());

            // Final attestation
            if i == att_per_check * 2 {
                // Before committing final attestation, queue should contain 2
                // checkpoints worth of attestations - 1
                assert_eq!(
                    Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
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
                    Attestation::checkpointing_queues(SUPPORTED_CHAIN_KEY).len(),
                    (att_per_check * 2) as usize
                );

                // Check that no attestations are missing from storage
                for attestation in &attestations {
                    assert_eq!(
                        Attestation::attestations(SUPPORTED_CHAIN_KEY, attestation.digest()),
                        Some(attestation.clone())
                    );
                }
            } else {
                // No checkpointing this pass
                assert_ok!(Attestation::commit_attestation(
                    attestor.stash.clone(),
                    attestation.clone()
                ));
            }
        }
    })
}

#[test]
fn removing_attestor_and_unbonding_staked_funds_work() {
    ExtBuilder.build_and_execute(|| {
        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        let min_bond_requirement = Attestation::min_bond_requirement(SUPPORTED_CHAIN_KEY);

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Unregister attestor
        assert_ok!(Attestation::unregister_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id
        ));

        let att = Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, ATTESTOR_1);
        assert!(att.is_none());

        // We are still staked
        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Get balance locks
        let locks = Balances::locks(&STASH_1);
        assert_eq!(locks.len(), 1);

        let locked_balance = Attestation::get_locked_balance(&attestor.stash_id);
        assert_eq!(locked_balance, min_bond_requirement);

        // Progress to block 50
        progress_to_block(50);

        // Withdraw unbonded
        assert_ok!(Attestation::withdraw_unbonded(attestor.stash));

        // We are no longer staked
        let ledger = Ledger::<Test>::get(STASH_1);
        // Ledger is nuked
        assert!(ledger.is_none());

        // Get balance locks
        let locks = Balances::locks(&STASH_1);
        assert_eq!(locks.len(), 0);

        let locked_balance = Attestation::get_locked_balance(&attestor.stash_id);
        assert_eq!(locked_balance, 0);

        System::assert_last_event(
            crate::Event::Withdrawn {
                stash: STASH_1,
                amount: 100_000_000_000_000_000_000,
            }
            .into(),
        );
    });
}

#[test]
fn withdrawing_unbonded_from_non_unregistered_attestors_fails() {
    ExtBuilder.build_and_execute(|| {
        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        let min_bond_requirement = Attestation::min_bond_requirement(SUPPORTED_CHAIN_KEY);

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Get balance locks
        let locks = Balances::locks(&STASH_1);
        assert_eq!(locks.len(), 1);

        // Progress to block 50
        progress_to_block(50);

        // Try to withdraw unbonded
        // Should do nothing since the attestor is not unregistered
        assert_ok!(Attestation::withdraw_unbonded(attestor.stash));

        // Get balance locks
        let locks = Balances::locks(&STASH_1);
        assert_eq!(locks.len(), 1);

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);
    });
}

#[test]
fn removing_attestor_and_withdrawing_fails_if_not_waited_long_enough() {
    ExtBuilder.build_and_execute(|| {
        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        let min_bond_requirement = Attestation::min_bond_requirement(SUPPORTED_CHAIN_KEY);

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Unregister attestor
        assert_ok!(Attestation::unregister_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id
        ));

        let att = Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, ATTESTOR_1);
        assert!(att.is_none());

        // We are still staked
        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Get balance locks
        let locks = Balances::locks(&STASH_1);
        assert_eq!(locks.len(), 1);

        // Progress to block 5
        progress_to_block(5);

        // Withdraw unbonded
        // Unbonding period is 2 eras, we are at era 1 so we should not be able to withdraw
        assert_ok!(Attestation::withdraw_unbonded(attestor.stash));

        // We are still staked
        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);
        assert_eq!(ledger.unlocking.len(), 1);
        assert_eq!(ledger.unlocking[0].era, 3);
    });
}

#[test]
fn unregistering_attestor_which_is_not_yours_fails() {
    ExtBuilder.build_and_execute(|| {
        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_noop!(
            Attestation::unregister_attestor(
                RuntimeOrigin::signed(STASH_2),
                SUPPORTED_CHAIN_KEY,
                attestor.attestor_id
            ),
            Error::<Test>::NotYourAttestor
        );
    });
}

#[test]
fn unregistering_non_existant_attestor_fails() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_attestor(
                RuntimeOrigin::signed(STASH_1),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ),
            Error::<Test>::AddressNotAttestor
        );
    });
}

#[test]
fn chilled_attestor_cannot_commit_attestation() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // Toggle to chilled
        assert_ok!(Attestation::chill(
            RuntimeOrigin::signed(STASH_1,),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(vec![attestor.clone()], 1, 1, None, None);

        let result = Attestation::validate_attestation(attestation.chain_key(), &attestation);
        assert_err!(result, Error::<Test>::AttestorNotActive);
    });
}

#[test]
fn withdraw_unbonded_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::withdraw_unbonded(RuntimeOrigin::none()),
            BadOrigin
        );
    })
}

#[test]
fn withdraw_unbonded_should_error_when_signer_is_not_a_stash() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::withdraw_unbonded(RuntimeOrigin::signed(STASH_1)),
            Error::<Test>::NotStash
        );
    })
}

#[test]
fn on_supported_chain_removed_cleans_up_storage_and_chills_attestors() {
    ExtBuilder.build_and_execute(|| {
        let dummy_val: u32 = 5000;

        // Set up attestors to be chilled
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        let attestor2 = Attestor::new(STASH_1, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            attestor2.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor2.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor2.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor2.public_key,
            attestor2.signature
        ));

        // Set up all storage items we want to remove:
        ActiveAttestors::<Test>::insert(SUPPORTED_CHAIN_KEY, vec![ATTESTOR_1]);
        Invulnerables::<Test>::insert(SUPPORTED_CHAIN_KEY, ATTESTOR_1, true);
        MaxAttestors::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val);
        MaxInvulnerables::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val);

        // Fill in attestations, checkpointing queue, and last digest
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let max_attestations =
            AttestationCheckpointInterval::<Test>::get(SUPPORTED_CHAIN_KEY) * 2 + 1; // +1 because very first attestation is kept
        for i in 0..max_attestations {
            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                SUPPORTED_CHAIN_KEY,
                (i * 10) as u64,
                None,
                None,
            );
            Attestations::<Test>::insert(
                SUPPORTED_CHAIN_KEY,
                attestation.digest(),
                attestation.clone(),
            );

            if i != 0 {
                // Very first attestation never goes in a checkpointing queue
                let mut queue = CheckpointingQueues::<Test>::get(SUPPORTED_CHAIN_KEY);
                queue.push_back(attestation.digest());
                CheckpointingQueues::<Test>::insert(SUPPORTED_CHAIN_KEY, queue);

                // Insert checkpoint
                let checkpoint = AttestationCheckpoint {
                    block_number: i as u64, // Mimic gap between checkpoint blocks
                    digest: attestation.digest(),
                };
                Checkpoints::<Test>::insert(SUPPORTED_CHAIN_KEY, i as u64, attestation.digest());
                LastCheckpoint::<Test>::insert(SUPPORTED_CHAIN_KEY, checkpoint);
            }
            if i == max_attestations - 1 {
                LastDigest::<Test>::insert(
                    SUPPORTED_CHAIN_KEY,
                    (attestation.header_number(), attestation.digest()),
                );
            }
        }

        PendingTargetSampleSize::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val);
        TargetSampleSize::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val);
        ChainAttestationInterval::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val as u64);
        PendingAttestationInterval::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val as u64);
        AttestationCheckpointInterval::<Test>::insert(SUPPORTED_CHAIN_KEY, dummy_val);

        // Remove supported chain
        assert_ok!(SupportedChains::remove_chain(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            true
        ));

        // Check that attestor 1 is chilled
        assert_eq!(
            Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, attestor.attestor_id)
                .unwrap()
                .status,
            AttestorStatus::Idle
        );
        // Check that attestor 2 is chilled
        assert_eq!(
            Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, attestor2.attestor_id)
                .unwrap()
                .status,
            AttestorStatus::Idle
        );

        // Check that storage items have been cleared
        assert_eq!(
            ActiveAttestors::<Test>::get(SUPPORTED_CHAIN_KEY),
            Vec::<mock::AccountId>::new()
        );
        assert_eq!(
            Invulnerables::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
            0
        );
        assert_eq!(
            MaxAttestors::<Test>::get(SUPPORTED_CHAIN_KEY),
            <Test as Config>::MaxAttestationNodes::get()
        );
        assert_eq!(
            MaxInvulnerables::<Test>::get(SUPPORTED_CHAIN_KEY),
            <Test as Config>::MaxAttestationNodes::get()
        );
        assert_eq!(
            Attestations::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
            0
        );
        assert_eq!(
            CheckpointingQueues::<Test>::get(SUPPORTED_CHAIN_KEY),
            Vec::<Digest>::new()
        );
        assert_eq!(LastDigest::<Test>::get(SUPPORTED_CHAIN_KEY), None);
        assert_eq!(
            PendingTargetSampleSize::<Test>::get(SUPPORTED_CHAIN_KEY),
            None
        );
        assert_eq!(
            TargetSampleSize::<Test>::get(SUPPORTED_CHAIN_KEY),
            <Test as Config>::DefaultTargetSampleSize::get()
        );
        assert_eq!(
            ChainAttestationInterval::<Test>::get(SUPPORTED_CHAIN_KEY),
            <Test as Config>::DefaultAttestationInterval::get()
        );
        assert_eq!(
            PendingAttestationInterval::<Test>::get(SUPPORTED_CHAIN_KEY),
            None
        );
        assert_eq!(
            AttestationCheckpointInterval::<Test>::get(SUPPORTED_CHAIN_KEY),
            <Test as Config>::DefaultAttestationsPerCheckpoint::get()
        );
        assert_eq!(LastCheckpoint::<Test>::get(SUPPORTED_CHAIN_KEY), None);

        System::assert_has_event(
            crate::Event::ClearedStorageForRemovedChain(SUPPORTED_CHAIN_KEY).into(),
        );
    })
}

// Repeated calls to clear_prefix within the same block do not stack. For this reason
// we need to commit storage overlays to backend storage between calls to clear_prefix.
// In doing so, we simulate the committing of changes at the end of a block.
#[extend::ext]
impl TestExternalities {
    fn run<F: FnOnce() -> R, R>(&mut self, f: F) -> R {
        let res = self.execute_with(f);
        self.commit_all().unwrap();
        res
    }
    fn then_run(&mut self, f: impl FnOnce()) -> &mut TestExternalities {
        self.run(f);
        self
    }
}

#[test]
fn on_supported_chain_removed_cleans_up_checkpoints() {
    let added_checkpoints = MAX_CHECKPOINTS_CLEARED_PER_BLOCK * 2 + 10;
    let mut test_ext = ExtBuilder.build();
    test_ext
        .then_run(|| {
            for i in 0..added_checkpoints {
                let checkpoint_digest = H256::from(&sp_io::hashing::blake2_256(&[i]));
                let checkpoint = AttestationCheckpoint {
                    block_number: i as u64 * 100, // Mimic gap between checkpoint blocks
                    digest: checkpoint_digest,
                };
                crate::Checkpoints::<Test>::insert(
                    SUPPORTED_CHAIN_KEY,
                    checkpoint.block_number,
                    checkpoint_digest,
                );
            }
            System::set_block_number(1);
            Timestamp::set_timestamp(1);

            assert_eq!(
                Checkpoints::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
                MAX_CHECKPOINTS_CLEARED_PER_BLOCK as usize * 2 + 10
            );
        })
        .then_run(|| {
            // Remove supported chain
            assert_ok!(SupportedChains::remove_chain(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                true
            ));

            assert_eq!(
                Checkpoints::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
                MAX_CHECKPOINTS_CLEARED_PER_BLOCK as usize + 10
            );
        })
        .then_run(|| {
            progress_to_block(2);

            assert_eq!(
                Checkpoints::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
                10
            );
        })
        .run(|| {
            progress_to_block(3);

            assert_eq!(
                Checkpoints::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count(),
                0
            );
            assert_eq!(
                CheckpointClearingCursors::<Test>::get(SUPPORTED_CHAIN_KEY),
                None
            );

            System::assert_last_event(crate::Event::CheckpointsCleared(SUPPORTED_CHAIN_KEY).into());
        })
}

#[test]
fn unregister_attestor_still_works_after_removing_that_attestors_chain() {
    ExtBuilder.build_and_execute(|| {
        // Set up attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // Check that attestor 1 is present and active
        assert_eq!(
            Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, attestor.attestor_id)
                .unwrap()
                .status,
            AttestorStatus::Waiting
        );

        // Remove supported chain
        assert_ok!(SupportedChains::remove_chain(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            true
        ));

        // Check that attestor 1 is present but inactive
        assert_eq!(
            Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, attestor.attestor_id)
                .unwrap()
                .status,
            AttestorStatus::Idle
        );

        assert_ok!(Attestation::unregister_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id
        ));

        assert_eq!(
            Attestors::<Test>::get(SUPPORTED_CHAIN_KEY, attestor.attestor_id),
            None
        );
    })
}

#[test]
fn batch_attestations_adding_one_on_duplicates_fails() {
    ExtBuilder.build_and_execute(|| {
        let attestor1 = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor1.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor1.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor1.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor1.public_key,
            attestor1.signature
        ));

        let attestor2 = Attestor::new(STASH_1, ATTESTOR_2);

        assert_ok!(Attestation::register_attestor(
            attestor2.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor2.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor2.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor2.public_key,
            attestor2.signature
        ));

        // We attested the previous block
        let attestation_block = System::block_number() - 1;

        progress_to_block(5);

        let attestation1 = create_signed_attestation(
            vec![attestor1.clone(), attestor2.clone()],
            SUPPORTED_CHAIN_KEY,
            0,
            None,
            None,
        );
        let attestation2 = create_signed_attestation(
            vec![attestor1.clone(), attestor2.clone()],
            SUPPORTED_CHAIN_KEY,
            10,
            Some(attestation1.digest()),
            None,
        );

        assert_ok!(Attestation::commit_attestation(
            attestor1.stash.clone(),
            attestation1.clone()
        ));

        assert_ok!(Attestation::commit_attestation(
            attestor1.stash.clone(),
            attestation2.clone()
        ));

        // Checkpoint is created for the first attestation
        let checkpoint_digest =
            Checkpoints::<Test>::get(SUPPORTED_CHAIN_KEY, attestation_block).unwrap();
        assert_eq!(checkpoint_digest, attestation1.digest());

        // Checkpoint buckets contains reference to checkpoint
        assert!(CheckpointBuckets::<Test>::contains_key((
            SUPPORTED_CHAIN_KEY,
            0,
            attestation_block
        )));

        let attestation3 = create_signed_attestation(
            vec![attestor1.clone(), attestor2.clone()],
            SUPPORTED_CHAIN_KEY,
            20,
            Some(attestation1.digest()),
            None,
        );

        // Duplicate attestation1
        let result = Attestation::validate_attestation(attestation1.chain_key(), &attestation1);
        assert_err!(result, Error::<Test>::AttestationExists);

        // Duplicate attestation2
        let result = Attestation::validate_attestation(attestation2.chain_key(), &attestation2);
        assert_err!(result, Error::<Test>::AttestationExists);

        // Add a new attestation
        let result = Attestation::validate_attestation(attestation3.chain_key(), &attestation3);
        assert_ok!(result);
    });
}

#[test]
fn setting_attestation_chain_genesis_block_number_works() {
    ExtBuilder.build_and_execute(|| {
        let genesis_block_number = 1000;

        assert_ok!(Attestation::set_attestation_chain_genesis_block_number(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            genesis_block_number
        ));

        let result = Attestation::attestation_chain_genesis_block_number(SUPPORTED_CHAIN_KEY);
        assert_eq!(result, genesis_block_number);
    });
}

#[test]
fn default_attestation_chain_genesis_block_number_works() {
    ExtBuilder.build_and_execute(|| {
        let genesis_block_number =
            <Test as Config>::DefaultAttestationChainGenesisBlockNumber::get();

        let result = Attestation::attestation_chain_genesis_block_number(SUPPORTED_CHAIN_KEY);
        assert_eq!(result, genesis_block_number);
    });
}

#[test]
fn set_attestation_chain_genesis_block_number_should_fail_when_not_root() {
    ExtBuilder.build_and_execute(|| {
        let genesis_block_number = 1000;

        assert_noop!(
            Attestation::set_attestation_chain_genesis_block_number(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                genesis_block_number
            ),
            BadOrigin
        );
    });
}

#[test]
fn set_attestation_chain_genesis_block_number_should_fail_when_chain_not_supported() {
    ExtBuilder.build_and_execute(|| {
        let genesis_block_number = 1000;

        // Attempt to set genesis block number for an unsupported chain
        assert_noop!(
            Attestation::set_attestation_chain_genesis_block_number(
                RuntimeOrigin::root(),
                123123, // Unsupported chain key
                genesis_block_number
            ),
            Error::<Test>::ChainNotSupported
        );
    });
}

#[test]
fn set_attestation_chain_genesis_block_number_should_fail_when_attestations_exist() {
    ExtBuilder.build_and_execute(|| {
        let genesis_block_number = 1000;
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // Register attestor and make them active
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        // Set genesis block number first
        AttestationChainGenesisBlockNumber::<Test>::insert(SUPPORTED_CHAIN_KEY, 0);

        // Create and commit an attestation
        let attestation =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);

        assert_ok!(Attestation::commit_attestation(
            attestor.stash,
            attestation.clone()
        ));

        // Verify attestation exists
        assert!(Attestations::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            attestation.digest()
        ));

        // Attempt to set genesis block number when attestations exist
        assert_noop!(
            Attestation::set_attestation_chain_genesis_block_number(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                genesis_block_number
            ),
            Error::<Test>::AttestationsAlreadyExist
        );
    });
}

#[test]
fn set_attestation_chain_genesis_block_number_should_fail_when_checkpoints_exist() {
    ExtBuilder.build_and_execute(|| {
        let genesis_block_number = 1000;

        // Manually insert a checkpoint
        let checkpoint_digest = H256::from([1; 32]);
        let checkpoint_block_number = 100;
        Checkpoints::<Test>::insert(
            SUPPORTED_CHAIN_KEY,
            checkpoint_block_number,
            checkpoint_digest,
        );

        // Verify checkpoint exists
        assert!(Checkpoints::<Test>::contains_key(
            SUPPORTED_CHAIN_KEY,
            checkpoint_block_number
        ));

        // Attempt to set genesis block number when checkpoints exist
        assert_noop!(
            Attestation::set_attestation_chain_genesis_block_number(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                genesis_block_number
            ),
            Error::<Test>::AttestationsAlreadyExist
        );
    });
}

#[test]
fn commit_attestation_should_fail_when_genesis_block_number_is_not_correct() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation =
            create_signed_attestation(vec![attestor.clone()], SUPPORTED_CHAIN_KEY, 0, None, None);

        // Set genesis block number to a different value
        AttestationChainGenesisBlockNumber::<Test>::insert(SUPPORTED_CHAIN_KEY, 2000);

        // Attempt to commit the attestation
        assert_noop!(
            Attestation::commit_attestation(attestor.stash, attestation.clone()),
            Error::<Test>::InvalidAttestationPrevDigest
        );
    });
}

#[test]
fn commit_attestation_should_succeed_when_genesis_block_number_is_correct() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            SUPPORTED_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(
            vec![attestor.clone()],
            SUPPORTED_CHAIN_KEY,
            10000,
            None,
            Some(AttestationFragment::default()),
        );

        // Set genesis block number to the correct value
        AttestationChainGenesisBlockNumber::<Test>::insert(SUPPORTED_CHAIN_KEY, 10000);

        // Attempt to commit the attestation
        assert_ok!(Attestation::commit_attestation(
            attestor.stash,
            attestation.clone()
        ));
    });
}

#[test]
fn set_vote_acceptance_window_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_vote_acceptance_window(RuntimeOrigin::none(), 2, 2),
            BadOrigin
        );
    })
}

#[test]
fn set_vote_acceptance_window_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_vote_acceptance_window(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                2
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_vote_acceptance_window_should_error_with_window_0() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_vote_acceptance_window(RuntimeOrigin::root(), SUPPORTED_CHAIN_KEY, 0),
            Error::<Test>::InvalidVoteAcceptanceWindow
        );
    })
}

#[test]
fn set_vote_acceptance_window_should_error_on_unsupported_chain() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 2;
        let vote_acceptance_window = 1;
        assert_noop!(
            Attestation::set_vote_acceptance_window(
                RuntimeOrigin::root(),
                chain_key,
                vote_acceptance_window
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn set_vote_acceptance_window_updates_internal_storage_and_emits_event() {
    ExtBuilder.build_and_execute(|| {
        let vote_acceptance_window = Attestation::vote_acceptance_window(SUPPORTED_CHAIN_KEY);
        assert_eq!(vote_acceptance_window, 3); // Window set in mock genesis

        let chain_vote_acceptance_window = 2;
        assert_ok!(Attestation::set_vote_acceptance_window(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            chain_vote_acceptance_window
        ));

        let vote_acceptance_window = Attestation::vote_acceptance_window(SUPPORTED_CHAIN_KEY);
        assert_eq!(vote_acceptance_window, 2);

        System::assert_last_event(
            crate::Event::VoteAcceptanceWindowChanged(
                SUPPORTED_CHAIN_KEY,
                chain_vote_acceptance_window,
            )
            .into(),
        );
    })
}

#[cfg(test)]
mod set_election_policy {
    use super::*;

    #[test]
    fn set_election_policy_should_error_when_not_signed() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::set_election_policy(
                    RuntimeOrigin::none(),
                    SUPPORTED_CHAIN_KEY,
                    AttestorElectionPolicy::DeniedToAll
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn set_election_policy_should_error_when_not_signed_by_root() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::set_election_policy(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    AttestorElectionPolicy::DeniedToAll
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn set_election_policy_should_update_storage_and_emit_event() {
        ExtBuilder.build_and_execute(|| {
            let initial_policy = Attestation::chain_election_policy(SUPPORTED_CHAIN_KEY);
            assert_eq!(initial_policy, AttestorElectionPolicy::OpenToAny);

            assert_ok!(Attestation::set_election_policy(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                AttestorElectionPolicy::DeniedToAll
            ));

            let updated_policy = Attestation::chain_election_policy(SUPPORTED_CHAIN_KEY);
            assert_eq!(updated_policy, AttestorElectionPolicy::DeniedToAll);

            System::assert_last_event(
                crate::Event::ChangedElectionPolicy(
                    SUPPORTED_CHAIN_KEY,
                    AttestorElectionPolicy::DeniedToAll,
                )
                .into(),
            );
        })
    }
}

#[cfg(test)]
mod authorize_attestor {
    use super::*;

    #[test]
    fn authorize_attestor_should_error_when_not_signed() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::authorize_attestor(
                    RuntimeOrigin::none(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn authorize_attestor_should_error_when_not_signed_by_root() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::authorize_attestor(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn authorize_attestor_should_error_when_chain_is_not_supported() {
        ExtBuilder.build_and_execute(|| {
            let unsupported_chain_key = 999;
            assert_noop!(
                Attestation::authorize_attestor(
                    RuntimeOrigin::root(),
                    unsupported_chain_key,
                    ATTESTOR_1
                ),
                Error::<Test>::ChainNotSupported
            );
        })
    }

    #[test]
    fn authorize_attestor_should_error_when_address_is_not_attestor() {
        ExtBuilder.build_and_execute(|| {
            // ATTESTOR_1 is not registered as an attestor yet
            assert_noop!(
                Attestation::authorize_attestor(
                    RuntimeOrigin::root(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                Error::<Test>::AddressNotAttestor
            );
        })
    }

    #[test]
    fn authorize_attestor_should_error_when_attestor_already_authorized() {
        ExtBuilder.build_and_execute(|| {
            // First, register an attestor
            let attestor = Attestor::new(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::register_attestor(
                attestor.stash.clone(),
                SUPPORTED_CHAIN_KEY,
                attestor.attestor_id,
            ));

            // Authorize the attestor
            assert_ok!(Attestation::authorize_attestor(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Try to authorize the same attestor again - should fail
            assert_noop!(
                Attestation::authorize_attestor(
                    RuntimeOrigin::root(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                Error::<Test>::AttestorAlreadyAuthorized
            );
        })
    }

    #[test]
    fn authorize_attestor_should_update_storage_and_emit_event() {
        ExtBuilder.build_and_execute(|| {
            // First, register an attestor
            let attestor = Attestor::new(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::register_attestor(
                attestor.stash.clone(),
                SUPPORTED_CHAIN_KEY,
                attestor.attestor_id,
            ));

            // Check that attestor is not yet authorized
            assert!(!AuthorizedAttestors::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Authorize the attestor
            assert_ok!(Attestation::authorize_attestor(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Check that attestor is now authorized
            assert!(AuthorizedAttestors::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Check that the event was emitted
            System::assert_last_event(
                crate::Event::AuthorizedAttestorAdded(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
            );
        })
    }
}

#[cfg(test)]
mod removed_authorized_attestor {
    use super::*;

    #[test]
    fn removed_authorized_attestor_should_error_when_not_signed() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::remove_authorized_attestor(
                    RuntimeOrigin::none(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn removed_authorized_attestor_should_error_when_not_signed_by_root() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::remove_authorized_attestor(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn removed_authorized_attestor_should_error_when_attestor_not_authorized() {
        ExtBuilder.build_and_execute(|| {
            // Try to remove authorization for an attestor that was never authorized
            assert_noop!(
                Attestation::remove_authorized_attestor(
                    RuntimeOrigin::root(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1
                ),
                Error::<Test>::AttestorNotAuthorized
            );
        })
    }

    #[test]
    fn removed_authorized_attestor_should_update_storage_and_emit_event() {
        ExtBuilder.build_and_execute(|| {
            // First, register an attestor
            let attestor = Attestor::new(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::register_attestor(
                attestor.stash.clone(),
                SUPPORTED_CHAIN_KEY,
                attestor.attestor_id,
            ));

            // Then authorize the attestor
            assert_ok!(Attestation::authorize_attestor(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Check that attestor is authorized
            assert!(AuthorizedAttestors::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Remove the authorization
            assert_ok!(Attestation::remove_authorized_attestor(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Check that attestor is no longer authorized
            assert!(!AuthorizedAttestors::<Test>::contains_key(
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1
            ));

            // Check that the event was emitted
            System::assert_last_event(
                crate::Event::AuthorizedAttestorRemoved(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
            );
        })
    }
}

#[cfg(test)]
mod kick_active_attestor {
    use super::*;

    #[test]
    fn kick_active_attestor_should_error_when_not_signed() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::kick_active_attestor(
                    RuntimeOrigin::none(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1,
                    false
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn kick_active_attestor_should_error_when_not_signed_by_root() {
        ExtBuilder.build_and_execute(|| {
            assert_noop!(
                Attestation::kick_active_attestor(
                    RuntimeOrigin::signed(ATTESTOR_1),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1,
                    false
                ),
                BadOrigin
            );
        })
    }

    #[test]
    fn kick_active_attestor_should_error_when_address_is_not_attestor() {
        ExtBuilder.build_and_execute(|| {
            // Try to kick an attestor that doesn't exist
            assert_noop!(
                Attestation::kick_active_attestor(
                    RuntimeOrigin::root(),
                    SUPPORTED_CHAIN_KEY,
                    ATTESTOR_1,
                    false
                ),
                Error::<Test>::AddressNotAttestor
            );
        })
    }

    #[test]
    fn kick_active_attestor_should_chill_attestor() {
        ExtBuilder.build_and_execute(|| {
            // First, register an attestor
            let attestor = Attestor::new(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::register_attestor(
                attestor.stash.clone(),
                SUPPORTED_CHAIN_KEY,
                attestor.attestor_id,
            ));

            // Activate the attestor
            assert_ok!(Attestation::attest(
                RuntimeOrigin::signed(attestor.attestor_id),
                SUPPORTED_CHAIN_KEY,
                attestor.public_key,
                attestor.signature
            ));

            progress_to_block(5);

            // Check that attestor is active
            let attestor_info = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
            assert_eq!(attestor_info.status, AttestorStatus::Active);

            assert!(ActiveAttestors::<Test>::get(SUPPORTED_CHAIN_KEY).contains(&ATTESTOR_1));

            // Kick the active attestor
            assert_ok!(Attestation::kick_active_attestor(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1,
                false
            ));

            // Check that attestor is now chilled
            let attestor_info = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
            assert_eq!(attestor_info.status, AttestorStatus::Idle);

            assert!(!ActiveAttestors::<Test>::get(SUPPORTED_CHAIN_KEY).contains(&ATTESTOR_1));

            // Check that the chilled event was emitted
            System::assert_has_event(
                crate::Event::AttestorChilled(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
            );
        })
    }

    #[test]
    fn kick_active_attestor_should_unregister_attestor_when_flag_is_set() {
        ExtBuilder.build_and_execute(|| {
            // First, register an attestor
            let attestor = Attestor::new(STASH_1, ATTESTOR_1);
            assert_ok!(Attestation::register_attestor(
                attestor.stash.clone(),
                SUPPORTED_CHAIN_KEY,
                attestor.attestor_id,
            ));

            // Activate the attestor
            assert_ok!(Attestation::attest(
                RuntimeOrigin::signed(attestor.attestor_id),
                SUPPORTED_CHAIN_KEY,
                attestor.public_key,
                attestor.signature
            ));

            progress_to_block(5);

            // Check that attestor is active
            let attestor_info = Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).unwrap();
            assert_eq!(attestor_info.status, AttestorStatus::Active);

            // Kick the active attestor with unregister flag set to true
            assert_ok!(Attestation::kick_active_attestor(
                RuntimeOrigin::root(),
                SUPPORTED_CHAIN_KEY,
                ATTESTOR_1,
                true
            ));

            // Check that attestor is no longer registered
            assert!(Attestation::attestors(SUPPORTED_CHAIN_KEY, ATTESTOR_1).is_none());

            // Check that the unregistered event was emitted
            System::assert_has_event(
                crate::Event::AttestorUnregistered(SUPPORTED_CHAIN_KEY, ATTESTOR_1).into(),
            );
        })
    }
}

#[test]
#[should_panic(expected = "InvalidBlsSignature")]
fn extract_agg_signature_should_fail_when_signature_is_invalid() {
    ExtBuilder.build_and_execute(|| {
        Attestation::extract_agg_signature(b"000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap();
    })
}

#[test]
#[should_panic(expected = "InvalidBlsSignature")]
fn verify_agg_signature_should_error_when_no_active_attestors() {
    ExtBuilder.build_and_execute(|| {
        let attestor1 = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor1], 1, 1, None, None);

        let attestor2 = Attestor::new(STASH_2, ATTESTOR_2);

        let agg_signature = Attestation::extract_agg_signature(&attestation.signature).unwrap();
        let message = &attestation.attestation.serialize()[..];
        Attestation::verify_agg_signature(
            &agg_signature,
            message,
            PublicKey::from_bytes(&attestor2.public_key).expect("failed"),
        )
        .unwrap();
    })
}

#[test]
#[should_panic(expected = "InvalidAttestorFound")]
fn gather_attestor_public_keys_should_error_when_attestor_not_registered() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        Attestation::gather_attestor_public_keys(SUPPORTED_CHAIN_KEY, &[attestor.attestor_id])
            .unwrap();
    })
}

#[test]
#[should_panic(expected = "AttestorWithInvalidPublicKey")]
fn gather_attestor_public_keys_should_error_when_bls_pubkey_malformed() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(STASH_1),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        crate::Attestors::<Test>::mutate(
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
            |attestor_item| {
                attestor_item.as_mut().unwrap().bls_public_key =
                    Some(*b"000000000000000000000000000000000000000000000000");
            },
        );

        Attestation::gather_attestor_public_keys(SUPPORTED_CHAIN_KEY, &[attestor.attestor_id])
            .unwrap();
    })
}

#[test]
#[should_panic(expected = "AttestorWithInvalidPublicKey")]
fn gather_attestor_public_keys_should_error_when_bls_pubkey_is_none() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(STASH_1),
            SUPPORTED_CHAIN_KEY,
            ATTESTOR_1
        ));

        crate::Attestors::<Test>::mutate(
            SUPPORTED_CHAIN_KEY,
            attestor.attestor_id,
            |attestor_item| {
                attestor_item.as_mut().unwrap().bls_public_key = None;
            },
        );

        Attestation::gather_attestor_public_keys(SUPPORTED_CHAIN_KEY, &[attestor.attestor_id])
            .unwrap();
    })
}

#[test]
fn import_checkpoints_works() {
    ExtBuilder.build_and_execute(|| {
        let checkpoints: Vec<AttestationCheckpoint> = vec![
            AttestationCheckpoint {
                block_number: 100,
                digest: [1u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 200,
                digest: [2u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 300,
                digest: [3u8; 32].into(),
            },
        ];

        assert_ok!(Attestation::import_checkpoints(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            checkpoints.clone().try_into().unwrap()
        ));

        for checkpoint in checkpoints {
            let stored_digest =
                Checkpoints::<Test>::get(SUPPORTED_CHAIN_KEY, checkpoint.block_number).unwrap();
            assert_eq!(stored_digest, checkpoint.digest);

            // Checkpoint buckets contains reference to checkpoint
            assert!(CheckpointBuckets::<Test>::contains_key((
                SUPPORTED_CHAIN_KEY,
                Pallet::<Test>::compute_block_index_for(checkpoint.block_number),
                checkpoint.block_number
            )));
        }
    });
}

#[test]
fn import_checkpoints_should_fail_when_not_root() {
    ExtBuilder.build_and_execute(|| {
        let checkpoints: Vec<AttestationCheckpoint> = vec![
            AttestationCheckpoint {
                block_number: 100,
                digest: [1u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 200,
                digest: [2u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 300,
                digest: [3u8; 32].into(),
            },
        ];

        assert_noop!(
            Attestation::import_checkpoints(
                RuntimeOrigin::signed(ATTESTOR_1),
                SUPPORTED_CHAIN_KEY,
                checkpoints.clone().try_into().unwrap()
            ),
            BadOrigin
        );
    });
}

#[test]
fn import_checkpoints_ignores_duplicate_digests() {
    ExtBuilder.build_and_execute(|| {
        let checkpoints: Vec<AttestationCheckpoint> = vec![
            AttestationCheckpoint {
                block_number: 100,
                digest: [1u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 100,
                digest: [1u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 200,
                digest: [2u8; 32].into(),
            },
        ];

        assert_ok!(Attestation::import_checkpoints(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            checkpoints.clone().try_into().unwrap()
        ));

        for checkpoint in checkpoints {
            let stored_digest =
                Checkpoints::<Test>::get(SUPPORTED_CHAIN_KEY, checkpoint.block_number).unwrap();
            assert_eq!(stored_digest, checkpoint.digest);

            // Checkpoint buckets contains reference to checkpoint
            assert!(CheckpointBuckets::<Test>::contains_key((
                SUPPORTED_CHAIN_KEY,
                Pallet::<Test>::compute_block_index_for(checkpoint.block_number),
                checkpoint.block_number
            )));
        }

        // Assert checkpoint total lengths
        let total_checkpoints = Checkpoints::<Test>::iter_prefix(SUPPORTED_CHAIN_KEY).count();
        assert_eq!(total_checkpoints, 2); // Only two unique digests
    });
}

#[test]
fn import_checkpoints_called_twice_works() {
    ExtBuilder.build_and_execute(|| {
        let checkpoints1: Vec<AttestationCheckpoint> = vec![
            AttestationCheckpoint {
                block_number: 100,
                digest: [1u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 200,
                digest: [2u8; 32].into(),
            },
        ];

        let checkpoints2: Vec<AttestationCheckpoint> = vec![
            AttestationCheckpoint {
                block_number: 300,
                digest: [3u8; 32].into(),
            },
            AttestationCheckpoint {
                block_number: 400,
                digest: [4u8; 32].into(),
            },
        ];

        assert_ok!(Attestation::import_checkpoints(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            checkpoints1.clone().try_into().unwrap()
        ));

        assert_ok!(Attestation::import_checkpoints(
            RuntimeOrigin::root(),
            SUPPORTED_CHAIN_KEY,
            checkpoints2.clone().try_into().unwrap()
        ));

        for checkpoint in checkpoints1.into_iter().chain(checkpoints2.into_iter()) {
            let stored_digest =
                Checkpoints::<Test>::get(SUPPORTED_CHAIN_KEY, checkpoint.block_number).unwrap();
            assert_eq!(stored_digest, checkpoint.digest);

            // Checkpoint buckets contains reference to checkpoint
            assert!(CheckpointBuckets::<Test>::contains_key((
                SUPPORTED_CHAIN_KEY,
                Pallet::<Test>::compute_block_index_for(checkpoint.block_number),
                checkpoint.block_number
            )));
        }
    });
}
