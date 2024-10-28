use super::*;
use crate::ledger::AttestorLedger;
use crate::mock::*;
use attestor_primitives::{
    Attestation as AttestationPrimitive, AttestationCheckpoint, AttestorStatus, ChainId,
    SignedAttestation,
};
use attestor_primitives::{BlsPublicKey, BlsSignature};
use bls_signatures::{aggregate, key::Serialize, PrivateKey};
use frame_support::{assert_noop, assert_ok};
use sp_core::H256;
use sp_runtime::traits::BadOrigin;

#[derive(Debug, Clone)]
pub struct Attestor {
    pub stash: RuntimeOrigin,
    pub stash_id: mock::AccountId,
    pub attestor_id: mock::AccountId,
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
fn set_min_bond_requirement_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_min_bond_requirement(RuntimeOrigin::none(), 200),
            BadOrigin
        );
    })
}

#[test]
fn set_min_bond_requirement_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_min_bond_requirement(RuntimeOrigin::signed(ATTESTOR_1), 200),
            BadOrigin
        );
    })
}

#[test]
fn set_min_bond_requirement_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let min_bond_requirement = Attestation::min_bond_requirement();
        assert_eq!(min_bond_requirement, 10_000);

        assert_ok!(Attestation::set_min_bond_requirement(
            RuntimeOrigin::root(),
            200
        ));

        let min_bond_requirement = Attestation::min_bond_requirement();
        assert_eq!(min_bond_requirement, 200);

        System::assert_last_event(crate::Event::MinBondRequirementUpdated(200).into());
    })
}

#[test]
fn set_chain_reward_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let chain_reward = 28;

        assert_noop!(
            Attestation::set_chain_reward(RuntimeOrigin::none(), DEV_CHAIN_KEY, chain_reward),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_reward_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let chain_reward = 28;

        assert_noop!(
            Attestation::set_chain_reward(
                RuntimeOrigin::signed(ATTESTOR_1),
                DEV_CHAIN_KEY,
                chain_reward
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_reward_should_error_when_chain_is_not_supported() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = ChainId::MAX;
        let chain_reward = 28;

        assert_noop!(
            Attestation::set_chain_reward(RuntimeOrigin::root(), chain_id, chain_reward),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn set_chain_reward_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_reward = 28;

        let from_storage = Attestation::chain_reward(DEV_CHAIN_KEY).unwrap();
        assert_eq!(from_storage, 10000); // from mock.rs genesis

        assert_ok!(Attestation::set_chain_reward(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            chain_reward
        ));

        let from_storage = Attestation::chain_reward(DEV_CHAIN_KEY).unwrap();
        assert_eq!(from_storage, chain_reward);

        System::assert_last_event(
            crate::Event::ChainRewardUpdated(DEV_CHAIN_KEY, chain_reward).into(),
        );
    })
}

#[test]
fn chill_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::chill(RuntimeOrigin::none(), DEV_CHAIN_KEY),
            BadOrigin
        );
    })
}

#[test]
fn chill_should_error_when_not_signed_by_an_attestor() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::chill(RuntimeOrigin::signed(ATTESTOR_1), DEV_CHAIN_KEY),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn chill_should_update_status_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // setup - register attestor
        assert_ok!(Attestation::register_attestor(
            RuntimeOrigin::signed(STASH_1),
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));

        // act
        assert_ok!(Attestation::chill(
            RuntimeOrigin::signed(ATTESTOR_1),
            DEV_CHAIN_KEY
        ));

        // assert
        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Idle);

        System::assert_last_event(crate::Event::AttestorChilled(DEV_CHAIN_KEY, ATTESTOR_1).into());
    })
}

#[test]
fn register_attestor_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));
        System::assert_last_event(
            crate::Event::AttestorRegistered(DEV_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn register_attestor_should_create_ledger_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        assert_eq!(attestor.bls_public_key, None);
        assert_eq!(attestor.status, AttestorStatus::Idle);

        let min_bond_requirement = MinBondRequirement::<Test>::get();

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // By default, the reward destination should be set to Stash
        let payee = Payee::<Test>::get(STASH_1);
        assert!(payee.is_some());
        let payee = payee.unwrap();
        assert_eq!(payee, RewardDestination::Stash);

        System::assert_last_event(
            crate::Event::AttestorRegistered(DEV_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn register_attestor_without_sufficient_funds_should_fail() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // Set min bond
        assert_ok!(Attestation::set_min_bond_requirement(
            RuntimeOrigin::root(),
            100_000_000_000
        ));

        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_noop!(
            Attestation::register_attestor(att.stash, DEV_CHAIN_KEY, att.attestor_id),
            Error::<Test>::InsufficientBalance
        );

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, 0);
    })
}

#[test]
fn register_attestor_without_sufficient_funds_should_fail_2() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let free_balance = Attestation::get_free_balance(&STASH_3);
        // 100_000 balance - 500 existential deposit
        assert_eq!(free_balance, 99500);

        // Set min bond
        // Balance of Stash 3 is 100_000
        assert_ok!(Attestation::set_min_bond_requirement(
            RuntimeOrigin::root(),
            60_000
        ));

        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let free_balance = Attestation::get_free_balance(&STASH_3);
        assert_eq!(free_balance, 39500);

        // We should not be able to register another attestor because we don't have enough funds
        let att = Attestor::new(STASH_3, ATTESTOR_2);
        assert_noop!(
            Attestation::register_attestor(att.stash, DEV_CHAIN_KEY, att.attestor_id),
            Error::<Test>::InsufficientBalance
        );
    })
}

#[test]
fn registering_multiple_attestor_increases_locked_balance() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let min_bond_requirement = MinBondRequirement::<Test>::get();

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement);

        // We should not be able to register another attestor because we don't have enough funds
        let att = Attestor::new(STASH_3, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 2);

        let att = Attestor::new(STASH_3, ATTESTOR_3);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 3);
    })
}

#[test]
fn registering_dergegistering_multiple_attestor_increases_decreases_locked_balance() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_3, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let min_bond_requirement = MinBondRequirement::<Test>::get();

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement);

        // We should not be able to register another attestor because we don't have enough funds
        let att = Attestor::new(STASH_3, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let locked_balance = Attestation::get_locked_balance(&STASH_3);
        assert_eq!(locked_balance, min_bond_requirement * 2);

        // deregister the second attestor
        assert_ok!(Attestation::unregister_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
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
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            DEV_CHAIN_KEY,
            att.public_key,
            att.signature
        ));
        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        // Public key should be set
        assert_eq!(attestor.bls_public_key, Some(att.public_key));
        assert_eq!(attestor.status, AttestorStatus::Active);

        // Chill
        assert_ok!(Attestation::chill(
            RuntimeOrigin::signed(att.attestor_id),
            DEV_CHAIN_KEY
        ));
        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Idle);
    })
}

#[test]
fn attestor_should_be_elected_after_5_blocks() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // Start attesting
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att.attestor_id),
            DEV_CHAIN_KEY,
            att.public_key,
            att.signature
        ));

        progress_to_block(5);

        assert!(Attestation::is_attestor(DEV_CHAIN_KEY, &ATTESTOR_1));
    })
}

#[test]
fn attestor_should_be_not_be_elected_after_5_blocks_if_not_signaling_start() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id
        ));

        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.stash, STASH_1);
        // Public key should be None
        assert_eq!(attestor.bls_public_key, None);
        // Default status should be Idle
        assert_eq!(attestor.status, AttestorStatus::Idle);

        progress_to_block(5);

        assert!(!Attestation::is_attestor(DEV_CHAIN_KEY, &ATTESTOR_1));
    })
}

#[test]
fn stash_should_be_able_to_set_payee() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        // assert_eq!(Attestors::<Test>::count(), 1);
        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));

        let payee = Payee::<Test>::get(STASH_1);
        assert!(payee.is_some());
        let payee = payee.unwrap();
        // Default payee should be Stash
        assert_eq!(payee, RewardDestination::Stash);

        assert_ok!(Attestation::set_payee(
            att.stash,
            RewardDestination::Account(STASH_2)
        ));

        let payee = Payee::<Test>::get(STASH_1);
        assert!(payee.is_some());
        let payee = payee.unwrap();
        // Payee should be updated to Account(STASH_2)
        assert_eq!(payee, RewardDestination::Account(STASH_2));
    })
}

#[test]
fn stash_ledger_schould_increase_when_registering_multiple_attestors() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let att = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let min_bond_requirement = MinBondRequirement::<Test>::get();

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        assert_eq!(ledger.total_staked, min_bond_requirement);

        let att = Attestor::new(STASH_1, ATTESTOR_2);
        assert_ok!(Attestation::register_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        assert_eq!(ledger.total_staked, min_bond_requirement * 2);

        let locks = Balances::locks(STASH_1);
        assert_eq!(locks.len(), 1);
        assert_eq!(locks[0].amount, min_bond_requirement * 2);
    })
}

#[test]
fn register_attestor_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_noop!(
            Attestation::register_attestor(RuntimeOrigin::none(), DEV_CHAIN_KEY, att.attestor_id,),
            BadOrigin
        );
    })
}

#[test]
fn register_attestor_should_error_when_address_is_already_registered() {
    ExtBuilder.build_and_execute(|| {
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        assert_noop!(
            Attestation::register_attestor(att.stash.clone(), DEV_CHAIN_KEY, att.attestor_id,),
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
        assert_ok!(Attestation::set_max_attestors(root, DEV_CHAIN_KEY, 1));
        assert_ok!(Attestation::register_attestor(
            att_1.stash,
            DEV_CHAIN_KEY,
            att_1.attestor_id,
        ));

        // note: test target is try_insert_attestor_and_emit_event()
        assert_noop!(
            Attestation::register_attestor(att_2.stash, DEV_CHAIN_KEY, att_2.attestor_id,),
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
                DEV_CHAIN_KEY,
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
                DEV_CHAIN_KEY,
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
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att.attestor_id),
                DEV_CHAIN_KEY,
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


        assert_ok!(Attestation::register_attestor(att.stash, DEV_CHAIN_KEY, att.attestor_id,));

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att.attestor_id),
                DEV_CHAIN_KEY,
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
            DEV_CHAIN_KEY,
            att1.attestor_id,
        ));

        assert_noop!(
            Attestation::attest(
                RuntimeOrigin::signed(att1.attestor_id),
                DEV_CHAIN_KEY,
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
            DEV_CHAIN_KEY,
            att1.attestor_id,
        ));
        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Idle);

        // act
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(att1.attestor_id),
            DEV_CHAIN_KEY,
            att1.public_key,
            att1.signature
        ),);

        // assert
        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).unwrap();
        assert_eq!(attestor.status, AttestorStatus::Active);
        assert_eq!(attestor.bls_public_key, Some(att1.public_key));

        System::assert_last_event(
            crate::Event::AttestorActivated(DEV_CHAIN_KEY, att1.attestor_id).into(),
        );
    })
}

// TODO: make this smarter and rely on the runtime value instead of the function
#[test]
fn max_attestor_default_should_be_100() {
    ExtBuilder.build_and_execute(|| assert_eq!(Attestation::max_attestors(DEV_CHAIN_KEY,), 100))
}

#[test]
fn max_invulnerable_default_should_be_100() {
    ExtBuilder.build_and_execute(|| assert_eq!(Attestation::max_invulnerables(DEV_CHAIN_KEY,), 100))
}

#[test]
fn set_max_invulnerables_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_max_invulnerables(RuntimeOrigin::none(), DEV_CHAIN_KEY, 200),
            BadOrigin
        );
    })
}

#[test]
fn set_max_invulnerables_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::set_max_invulnerables(bad_origin, DEV_CHAIN_KEY, 200),
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
            Attestation::set_max_invulnerables(root_origin, DEV_CHAIN_KEY, 0),
            Error::<Test>::MaxInvulnerablesCannotBeChanged
        );
    })
}

#[test]
fn set_max_invulnerables_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        assert_eq!(Attestation::max_invulnerables(DEV_CHAIN_KEY,), 100);
        let count = Invulnerables::<Test>::iter_prefix_values(DEV_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 1); // from mock

        assert_ok!(Attestation::set_max_invulnerables(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            10
        ),);
        assert_eq!(Attestation::max_invulnerables(DEV_CHAIN_KEY,), 10);
        let count = Invulnerables::<Test>::iter_prefix_values(DEV_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 1); // from mock
    })
}

#[test]
fn set_max_attestors_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_max_attestors(RuntimeOrigin::none(), DEV_CHAIN_KEY, 1),
            BadOrigin
        );
    })
}

#[test]
fn set_max_attestors_should_error_with_non_root_origin() {
    ExtBuilder.build_and_execute(|| {
        let bad_origin = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::set_max_attestors(bad_origin, DEV_CHAIN_KEY, 1),
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
            DEV_CHAIN_KEY,
            att_1.attestor_id,
        ));
        assert_ok!(Attestation::register_attestor(
            att_2.stash,
            DEV_CHAIN_KEY,
            att_2.attestor_id,
        ));

        let count = Attestors::<Test>::iter_prefix_values(DEV_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 2);

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            1
        ));
        let count = Attestors::<Test>::iter_prefix_values(DEV_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 2);
        let max_attestors = Attestation::max_attestors(DEV_CHAIN_KEY);
        assert_eq!(max_attestors, 1);
    })
}

#[test]
fn set_max_attestors_should_work_when_list_is_empty() {
    ExtBuilder.build_and_execute(|| {
        let _ = Attestors::<Test>::clear(u32::MAX, None);
        let count = Attestors::<Test>::iter_prefix_values(DEV_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 0);

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            5
        ));
        let max_attestors = Attestation::max_attestors(DEV_CHAIN_KEY);
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
            DEV_CHAIN_KEY,
            att_1.attestor_id,
        ));
        assert_ok!(Attestation::register_attestor(
            att_2.stash,
            DEV_CHAIN_KEY,
            att_2.attestor_id,
        ));

        let count = Attestors::<Test>::iter_prefix_values(DEV_CHAIN_KEY)
            .collect::<Vec<_>>()
            .len();
        assert_eq!(count, 2);
        // this is the default value
        let max_attestors = Attestation::max_attestors(DEV_CHAIN_KEY);
        assert_eq!(max_attestors, 100);

        assert_ok!(Attestation::set_max_attestors(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            10
        ),);
        let max_attestors = Attestation::max_attestors(DEV_CHAIN_KEY);
        assert_eq!(max_attestors, 10);
    })
}

#[test]
fn unregister_attestor_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_attestor(RuntimeOrigin::none(), DEV_CHAIN_KEY, ATTESTOR_1),
            BadOrigin
        );
    })
}

#[test]
fn unregister_attestor_should_error_when_address_is_not_registered_as_attestor() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);
        assert_noop!(
            Attestation::unregister_attestor(attestor, DEV_CHAIN_KEY, ATTESTOR_1),
            Error::<Test>::AddressNotAttestor
        );
    })
}

#[test]
fn unregister_attestor_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // setup
        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            att.stash.clone(),
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));
        assert!(Attestation::attestor_is_registered(
            DEV_CHAIN_KEY,
            &ATTESTOR_1
        ));

        // test
        assert_ok!(Attestation::unregister_attestor(
            att.stash,
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));
        let attestor = Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1);
        assert!(attestor.is_none());
        System::assert_last_event(
            crate::Event::AttestorUnregistered(DEV_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

#[test]
fn unregister_invulnerable_should_update_storage_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // setup
        assert!(!Invulnerables::<Test>::contains_key(
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));

        let att = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));
        assert!(Attestation::invulnerables(DEV_CHAIN_KEY, ATTESTOR_1).is_some());

        // test
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));
        assert!(!Invulnerables::<Test>::contains_key(
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));
        System::assert_last_event(
            crate::Event::InvulnerableUnregistered(DEV_CHAIN_KEY, ATTESTOR_1).into(),
        )
    })
}

#[test]
fn unregister_invulnerable_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::none(), DEV_CHAIN_KEY, ATTESTOR_1),
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
                DEV_CHAIN_KEY,
                ATTESTOR_1
            ),
            BadOrigin
        );
    })
}

#[test]
fn unregister_invulnerable_should_fail_when_address_is_not_registered_at_all() {
    ExtBuilder.build_and_execute(|| {
        assert!(!Attestors::<Test>::contains_key(DEV_CHAIN_KEY, ATTESTOR_1));
        assert!(!Invulnerables::<Test>::contains_key(
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), DEV_CHAIN_KEY, ATTESTOR_1),
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
            DEV_CHAIN_KEY,
            att.attestor_id,
        ));
        assert!(Attestation::attestors(DEV_CHAIN_KEY, ATTESTOR_1).is_some());
        assert!(!Invulnerables::<Test>::contains_key(
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_noop!(
            Attestation::unregister_invulnerable(RuntimeOrigin::root(), DEV_CHAIN_KEY, ATTESTOR_1),
            Error::<Test>::AddressIsNotInvulnerable
        );
    })
}

#[test]
fn set_comittee_set_size_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            Attestation::set_comittee_set_size(RuntimeOrigin::none(), DEV_CHAIN_KEY, 512),
            BadOrigin
        );
    })
}

#[test]
fn set_comittee_set_size_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = RuntimeOrigin::signed(ATTESTOR_1);

        assert_noop!(
            Attestation::set_comittee_set_size(attestor, DEV_CHAIN_KEY, 512),
            BadOrigin
        );
    })
}

#[test]
fn set_comittee_set_size_should_update_storage_and_emit_an_event() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let comittee_size = Attestation::comittee_set_size(DEV_CHAIN_KEY);
        assert_eq!(comittee_size, DEFAULT_COMITTEE_SET_SIZE);

        let new_comittee_size = 512;
        assert_ok!(Attestation::set_comittee_set_size(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            new_comittee_size
        ));

        let comittee_size = Attestation::comittee_set_size(DEV_CHAIN_KEY);
        assert_eq!(comittee_size, new_comittee_size);

        System::assert_last_event(
            crate::Event::ComitteeSetSizeChanged(DEV_CHAIN_KEY, new_comittee_size).into(),
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
                DEV_CHAIN_KEY,
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
                DEV_CHAIN_KEY,
                att.attestor_id,
            ),
            BadOrigin
        );
    })
}

#[test]
fn register_invulnerable_adds_attestor_and_invulnerable_and_emits_events() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert!(!Invulnerables::<Test>::contains_key(
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));

        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            ATTESTOR_1,
        ));

        assert!(Attestation::invulnerables(DEV_CHAIN_KEY, ATTESTOR_1).is_some());

        // assert on event
        System::assert_last_event(
            crate::Event::InvulnerableRegistered(DEV_CHAIN_KEY, ATTESTOR_1).into(),
        );
    })
}

// Rare case that an invulnerable signals unregister and then sudo removes that one as invulnerable
#[test]
fn remove_invulnerable_works() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(Attestation::register_invulnerable(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            ATTESTOR_1,
        ));

        // Still invulnerable
        assert!(Attestation::invulnerables(DEV_CHAIN_KEY, ATTESTOR_1).is_some());

        // Remove as invulnerable
        assert_ok!(Attestation::unregister_invulnerable(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            ATTESTOR_1
        ));
    })
}

#[test]
fn set_chain_attestation_interval_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_attestation_interval = 101;

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::none(),
                DEV_CHAIN_KEY,
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

        let chain_attestation_interval = 101;

        let acct: AccountId = 4;

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::signed(acct),
                DEV_CHAIN_KEY,
                chain_attestation_interval
            ),
            BadOrigin
        );
    })
}

#[test]
fn set_chain_attestation_interval_should_error_with_interval_0() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let chain_attestation_interval = 0;
        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::root(),
                chain_id,
                chain_attestation_interval
            ),
            Error::<Test>::InvalidAttestationInterval
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
fn set_chain_attestation_interval_updates_internal_storage_and_emits_event() {
    ExtBuilder.build_and_execute(|| {
        let attestation_interval = Attestation::chain_attestation_interval(DEV_CHAIN_KEY);
        assert_eq!(attestation_interval, 10); // Interval set in mock genesis

        let chain_attestation_interval = 101;
        assert_ok!(Attestation::set_chain_attestation_interval(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            chain_attestation_interval
        ));

        let attestation_interval = Attestation::chain_attestation_interval(DEV_CHAIN_KEY);
        assert_eq!(attestation_interval, 101);

        let chain_id = SupportedChains::supported_chain(DEV_CHAIN_KEY)
            .expect("Checked that this chain was supported when setting interval.")
            .chain_id;

        System::assert_last_event(
            crate::Event::AttestationIntervalChanged(chain_id, chain_attestation_interval, 0)
                .into(),
        );
    })
}

#[test]
fn set_chain_attestation_interval_fails_when_attestation_matching_last_digest_is_not_found() {
    ExtBuilder.build_and_execute(|| {
        let chain_attestation_interval = 101;

        // Setting last digest without inserting a corresponding attestation.
        let digest: H256 = [0u8; 32].into();
        LastDigest::<Test>::insert(DEV_CHAIN_KEY, digest);

        assert_noop!(
            Attestation::set_chain_attestation_interval(
                RuntimeOrigin::root(),
                DEV_CHAIN_KEY,
                chain_attestation_interval
            ),
            Error::<Test>::AttestationNotFound
        );
    })
}

#[test]
fn set_attestations_per_checkpoint_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        let att_per_check = Attestation::attestation_checkpoint_interval(DEV_CHAIN_KEY);
        assert_eq!(att_per_check, 10); // Checkpoint frequencty set in mock genesis

        let new_att_per_check = 101;
        assert_ok!(Attestation::set_attestations_per_checkpoint(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            new_att_per_check
        ));

        let att_per_check = Attestation::attestation_checkpoint_interval(DEV_CHAIN_KEY);
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
                DEV_CHAIN_KEY,
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
            Attestation::set_attestations_per_checkpoint(RuntimeOrigin::root(), DEV_CHAIN_KEY, 0),
            Error::<Test>::InvalidAttestationsPerCheckpoint
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
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_noop!(
            Attestation::bootstrap_chain(RuntimeOrigin::none(), DEV_CHAIN_KEY, attestation,),
            BadOrigin
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);

        assert_noop!(
            Attestation::bootstrap_chain(
                RuntimeOrigin::signed(ATTESTOR_1),
                DEV_CHAIN_KEY,
                attestation,
            ),
            BadOrigin
        );
    })
}

#[test]
fn bootstrap_chain_should_error_when_chain_is_unsupported() {
    ExtBuilder.build_and_execute(|| {
        let chain_id = 2;
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
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

        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let attestation = create_signed_attestation(vec![attestor], 30, 1, None);
        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation.header_number(),
            digest: attestation.digest(),
        };

        assert_eq!(Attestation::last_attestation_digest(DEV_CHAIN_KEY), None);
        assert_eq!(
            Attestation::attestations(DEV_CHAIN_KEY, attestation.digest()),
            None
        );
        assert_eq!(Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(), 0);

        assert_ok!(Attestation::bootstrap_chain(
            RuntimeOrigin::root(),
            DEV_CHAIN_KEY,
            attestation.clone(),
        ),);

        // storage
        assert_eq!(
            Attestation::last_attestation_digest(DEV_CHAIN_KEY),
            Some(attestation.digest())
        );
        assert_eq!(
            Attestation::attestations(DEV_CHAIN_KEY, attestation.digest()),
            Some(attestation.clone())
        );
        // Shouldn't add first attestation for chain to checkpointing queue
        assert_eq!(Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(), 0);

        // event
        System::assert_last_event(
            crate::Event::CheckpointReached(DEV_CHAIN_KEY, expected_checkpoint).into(),
        );
    })
}

#[test]
fn commit_attestation_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(), 0);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(vec![attestor], DEV_CHAIN_KEY, 1, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        // The first attestation for a chain immediately creates a corresponding checkpoint
        // rather than adding to the checkpointing queue.
        let expected_checkpoint = AttestationCheckpoint {
            block_number: attestation.header_number(),
            digest: attestation.digest(),
        };
        assert_eq!(
            Attestation::checkpoints(DEV_CHAIN_KEY, expected_checkpoint.digest),
            Some(expected_checkpoint)
        );

        assert_eq!(
            Attestation::attestations(DEV_CHAIN_KEY, attestation.digest()),
            Some(attestation)
        );
    })
}

#[test]
fn commit_attestation_should_error_when_signed() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
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
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
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

        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
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
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

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
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
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
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_eq!(Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(), 0);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation_1 =
            create_signed_attestation(vec![attestor.clone()], DEV_CHAIN_KEY, 1, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation_1.clone()
        ));

        let digest = attestation_1.digest();

        let attestation_2 =
            create_signed_attestation(vec![attestor], DEV_CHAIN_KEY, 11, Some(digest));

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation_2.clone()
        ));

        // Only second attestation should have been added to a queue
        assert_eq!(Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(), 1);
        assert_eq!(
            Attestation::checkpointing_queues(DEV_CHAIN_KEY).back(),
            Some(&attestation_2.digest())
        );
        assert_eq!(
            Attestation::attestations(DEV_CHAIN_KEY, attestation_1.digest()),
            Some(attestation_1)
        );
        assert_eq!(
            Attestation::attestations(DEV_CHAIN_KEY, attestation_2.digest()),
            Some(attestation_2)
        );
    })
}

#[test]
fn submitting_invalid_attestation_chain_fails() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

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
        System::set_block_number(1);
        // Setup almost two full checkpoints of attestations, so that
        // the next attestation submitted triggers checkpoint creation.
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        let att_interval = Attestation::chain_attestation_interval(DEV_CHAIN_KEY);
        let att_per_check = Attestation::attestation_checkpoint_interval(DEV_CHAIN_KEY);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let mut last_digest: Option<H256> = None;
        let mut removed_by_checkpoint: Vec<H256> = Vec::new();
        let mut kept_after_checkpoint: Vec<SignedAttestation<H256, u64>> = Vec::new();
        let mut checkpoint_attestation: Option<SignedAttestation<H256, u64>> = None;
        for i in 0..(att_per_check * 2 + 1) as usize {
            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                DEV_CHAIN_KEY,
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
            Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(),
            att_per_check as usize
        );

        for removed_digest in removed_by_checkpoint {
            assert_eq!(
                Attestation::attestations(DEV_CHAIN_KEY, removed_digest),
                None
            );
        }

        for kept_attestation in kept_after_checkpoint {
            assert_eq!(
                Attestation::attestations(DEV_CHAIN_KEY, kept_attestation.digest()),
                Some(kept_attestation)
            )
        }

        let unwrapped_att =
            checkpoint_attestation.expect("Should have been filled to Some in loop.");
        let resulting_checkpoint = AttestationCheckpoint {
            block_number: unwrapped_att.header_number(),
            digest: unwrapped_att.digest(),
        };
        System::assert_last_event(
            crate::Event::CheckpointReached(DEV_CHAIN_KEY, resulting_checkpoint.clone()).into(),
        );
        assert_eq!(
            Attestation::checkpoints(DEV_CHAIN_KEY, resulting_checkpoint.digest),
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
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        let att_interval = Attestation::chain_attestation_interval(DEV_CHAIN_KEY);
        let att_per_check = Attestation::attestation_checkpoint_interval(DEV_CHAIN_KEY) as u64;

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let mut last_digest: Option<H256> = None;
        let mut attestations = Vec::new();
        // Add initial attestation, which belongs to its own special checkpoint interval,
        // as well as all but 1 of the attestations in the following interval.
        for i in 0..att_per_check {
            let attestation = create_signed_attestation(
                vec![attestor.clone()],
                DEV_CHAIN_KEY,
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
                DEV_CHAIN_KEY,
                (att_interval * i) + 1,
                last_digest,
            );
            last_digest = Some(attestation.digest());

            // Final attestation
            if i == att_per_check * 2 {
                // Before committing final attestation, queue should contain 2
                // checkpoints worth of attestations - 1
                assert_eq!(
                    Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(),
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
                    Attestation::checkpointing_queues(DEV_CHAIN_KEY).len(),
                    (att_per_check * 2) as usize
                );

                // Check that no attestations are missing from storage
                for attestation in &attestations {
                    assert_eq!(
                        Attestation::attestations(DEV_CHAIN_KEY, attestation.digest()),
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

#[test]
fn removing_attestor_and_unbonding_staked_funds_work() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        let min_bond_requirement = Attestation::min_bond_requirement();

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Unregister attestor
        assert_ok!(Attestation::unregister_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id
        ));

        let att = Attestors::<Test>::get(DEV_CHAIN_KEY, ATTESTOR_1);
        assert!(att.is_none());

        // We are still staked
        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Get balance locks
        let locks = Balances::locks(STASH_1);
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
        let locks = Balances::locks(STASH_1);
        assert_eq!(locks.len(), 0);

        let locked_balance = Attestation::get_locked_balance(&attestor.stash_id);
        assert_eq!(locked_balance, 0);

        System::assert_last_event(
            crate::Event::Withdrawn {
                stash: STASH_1,
                amount: 10000,
            }
            .into(),
        );
    });
}

#[test]
fn withdrawing_unbonded_from_non_unregistered_attestors_fails() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        let min_bond_requirement = Attestation::min_bond_requirement();

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Get balance locks
        let locks = Balances::locks(STASH_1);
        assert_eq!(locks.len(), 1);

        // Progress to block 50
        progress_to_block(50);

        // Try to withdraw unbonded
        // Should do nothing since the attestor is not unregistered
        assert_ok!(Attestation::withdraw_unbonded(attestor.stash));

        // Get balance locks
        let locks = Balances::locks(STASH_1);
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
        System::set_block_number(1);

        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        let min_bond_requirement = Attestation::min_bond_requirement();

        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Unregister attestor
        assert_ok!(Attestation::unregister_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id
        ));

        let att = Attestors::<Test>::get(DEV_CHAIN_KEY, ATTESTOR_1);
        assert!(att.is_none());

        // We are still staked
        let ledger = Ledger::<Test>::get(STASH_1);
        assert!(ledger.is_some());
        let ledger = ledger.unwrap();
        assert_eq!(ledger.stash, STASH_1);
        // The total staked amount should be equal to the min bond requirement
        assert_eq!(ledger.total_staked, min_bond_requirement);

        // Get balance locks
        let locks = Balances::locks(STASH_1);
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
        System::set_block_number(1);

        // register attestor
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        assert_noop!(
            Attestation::unregister_attestor(
                RuntimeOrigin::signed(STASH_2),
                DEV_CHAIN_KEY,
                attestor.attestor_id
            ),
            Error::<Test>::NotYourAttestor
        );
    });
}

#[test]
fn unregistering_non_existant_attestor_fails() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            Attestation::unregister_attestor(
                RuntimeOrigin::signed(STASH_1),
                DEV_CHAIN_KEY,
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
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // Toggle to chilled
        assert_ok!(Attestation::chill(
            RuntimeOrigin::signed(attestor.attestor_id,),
            DEV_CHAIN_KEY
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(vec![attestor.clone()], 1, 1, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::InvalidAttestation
        );
    });
}

#[test]
fn unregistered_attestor_cannot_commit_attestation() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // Chill
        assert_ok!(Attestation::chill(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY
        ));

        // Unregister attestor
        assert_ok!(Attestation::unregister_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id
        ));

        let attestation = create_signed_attestation(vec![attestor.clone()], 1, 1, None);

        assert_noop!(
            Attestation::commit_attestation(RuntimeOrigin::none(), attestation),
            Error::<Test>::InvalidAttestation
        );
    });
}

#[test]
fn accumulating_rewards_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(vec![attestor.clone()], 1, 0, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        // Get reward for chain 1
        let chain_reward = ChainReward::<Test>::get(1).unwrap();

        // Check that the reward was paid
        System::assert_has_event(
            crate::Event::RewardPaid {
                chain_id: DEV_CHAIN_KEY,
                stash: STASH_1,
                amount: chain_reward,
            }
            .into(),
        );

        let rewards = AccumulatedRewards::<Test>::get(attestor.stash_id);
        assert!(rewards.is_some());

        let rewards = rewards.unwrap();
        assert_eq!(rewards, chain_reward);

        let attestation =
            create_signed_attestation(vec![attestor.clone()], 1, 10, Some(attestation.digest()));

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation
        ));

        let rewards = AccumulatedRewards::<Test>::get(attestor.stash_id);
        assert!(rewards.is_some());

        let rewards = rewards.unwrap();
        assert_eq!(rewards, chain_reward * 2);

        // Check that the reward was paid
        System::assert_has_event(
            crate::Event::RewardPaid {
                chain_id: DEV_CHAIN_KEY,
                stash: STASH_1,
                amount: chain_reward,
            }
            .into(),
        );
    });
}

#[test]
fn accumulating_rewards_with_multiple_attestors_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        let attestor2 = Attestor::new(STASH_1, ATTESTOR_2);

        assert_ok!(Attestation::register_attestor(
            attestor2.stash.clone(),
            DEV_CHAIN_KEY,
            attestor2.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor2.attestor_id),
            DEV_CHAIN_KEY,
            attestor2.public_key,
            attestor2.signature
        ));

        progress_to_block(5);

        let attestation =
            create_signed_attestation(vec![attestor.clone(), attestor2.clone()], 1, 0, None);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        // Get reward for chain 1
        let chain_reward = ChainReward::<Test>::get(1).unwrap();

        let rewards = AccumulatedRewards::<Test>::get(attestor.stash_id);
        assert!(rewards.is_some());

        let rewards = rewards.unwrap();
        assert_eq!(rewards, chain_reward * 2);

        let attestation = create_signed_attestation(
            vec![attestor.clone(), attestor2.clone()],
            1,
            10,
            Some(attestation.digest()),
        );

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation
        ));

        let rewards = AccumulatedRewards::<Test>::get(attestor.stash_id);
        assert!(rewards.is_some());

        let rewards = rewards.unwrap();
        assert_eq!(rewards, chain_reward * 4);
    });
}

#[test]
fn claim_rewards_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(Attestation::claim_rewards(RuntimeOrigin::none()), BadOrigin);
    })
}

#[test]
fn claim_rewards_should_error_when_no_reward() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance = Balances::free_balance(STASH_1);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // there are no rewards b/c this attestor has not committed any attestations
        assert_noop!(
            Attestation::claim_rewards(attestor.stash),
            Error::<Test>::NoRewards
        );

        let new_balance = Balances::free_balance(STASH_1);
        assert_eq!(new_balance, initial_balance);
    });
}

#[test]
fn claim_rewards_should_not_update_balance_when_reward_is_zero() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance = Balances::free_balance(STASH_1);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // simulate a zero reward being accumulated w/o committing an attestation
        AccumulatedRewards::<Test>::insert(STASH_1, 0);

        assert_ok!(Attestation::claim_rewards(attestor.stash),);

        let new_balance = Balances::free_balance(STASH_1);
        assert_eq!(new_balance, initial_balance);
    });
}

#[test]
fn claim_rewards_should_not_update_balance_when_payee_is_none() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance = Balances::free_balance(STASH_1);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // explicitly configure that we don't want to be paid out any rewards
        assert_ok!(Attestation::set_payee(
            attestor.stash.clone(),
            RewardDestination::None
        ));

        // simulate a reward being accumulated w/o committing an attestation
        AccumulatedRewards::<Test>::insert(STASH_1, 500);

        assert_ok!(Attestation::claim_rewards(attestor.stash),);

        let new_balance = Balances::free_balance(STASH_1);
        assert_eq!(new_balance, initial_balance);
    });
}

#[test]
fn claim_rewards_should_update_balance_when_payee_is_another_account() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance_for_stash_1 = Balances::free_balance(STASH_1);
        let initial_balance_for_stash_2 = Balances::free_balance(STASH_2);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // explicitly configure reward to be paid out to someone else
        assert_ok!(Attestation::set_payee(
            attestor.stash.clone(),
            RewardDestination::Account(STASH_2),
        ));

        // simulate a reward being accumulated w/o committing an attestation
        AccumulatedRewards::<Test>::insert(STASH_1, 5000);

        assert_ok!(Attestation::claim_rewards(attestor.stash),);

        // assert that reward was paid to STASH_2, not STASH_1
        let new_balance_for_stash_1 = Balances::free_balance(STASH_1);
        let new_balance_for_stash_2 = Balances::free_balance(STASH_2);
        assert_eq!(new_balance_for_stash_1, initial_balance_for_stash_1);
        assert_eq!(new_balance_for_stash_2, initial_balance_for_stash_2 + 5000);

        // emitted event is still related to STASH_1 b/c they are the one
        // who accumulated and claimed the rewards
        System::assert_last_event(
            crate::Event::RewardClaimed {
                stash: STASH_1,
                amount: 5000,
            }
            .into(),
        );
    });
}

#[test]
fn claim_rewards_should_update_balance_when_payee_is_stash() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance = Balances::free_balance(STASH_1);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // explicitly configure that we want reward to be paid out to the stash account
        assert_ok!(Attestation::set_payee(
            attestor.stash.clone(),
            RewardDestination::Stash,
        ));

        // simulate a reward being accumulated w/o committing an attestation
        AccumulatedRewards::<Test>::insert(STASH_1, 4000);

        assert_ok!(Attestation::claim_rewards(attestor.stash),);

        // assert that reward was paid to STASH_1
        let new_balance = Balances::free_balance(STASH_1);
        assert_eq!(new_balance, initial_balance + 4000);

        // emitted event is related to STASH_1
        System::assert_last_event(
            crate::Event::RewardClaimed {
                stash: STASH_1,
                amount: 4000,
            }
            .into(),
        );
    });
}

#[test]
fn claim_rewards_should_update_stash_balance_when_payee_not_found() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance = Balances::free_balance(STASH_1);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        // note: not calling set_payee(), payouts default to STASH_1
        // and removing this explicitly b/c it is set in ledger.bond()
        // in order to exercise impls.rs:333 (payee not found)
        Payee::<Test>::remove(STASH_1);

        // simulate a reward being accumulated w/o committing an attestation
        AccumulatedRewards::<Test>::insert(STASH_1, 6000);

        assert_ok!(Attestation::claim_rewards(attestor.stash),);

        // assert that reward was paid to STASH_1
        let new_balance = Balances::free_balance(STASH_1);
        assert_eq!(new_balance, initial_balance + 6000);

        // emitted event is related to STASH_1
        System::assert_last_event(
            crate::Event::RewardClaimed {
                stash: STASH_1,
                amount: 6000,
            }
            .into(),
        );
    });
}

#[test]
fn claim_rewards_should_update_balance_and_emit_event() {
    ExtBuilder.build_and_execute(|| {
        let initial_balance = Balances::free_balance(STASH_1);
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        // setup - register and activate the attestor; commit an attestation
        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        let attestation = create_signed_attestation(vec![attestor.clone()], 1, 0, None);

        progress_to_block(5);

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        // reward for chain 1 configured in mock.rs
        let chain_reward = ChainReward::<Test>::get(1).unwrap();

        let rewards = AccumulatedRewards::<Test>::get(attestor.stash_id);
        assert!(rewards.is_some());

        let rewards = rewards.unwrap();
        // assert - accumulated reward equals the configured one
        assert_eq!(rewards, chain_reward);

        // act - claim the accumulated reward - updates balance and emits event
        assert_ok!(Attestation::claim_rewards(attestor.stash));

        let new_balance = Balances::free_balance(STASH_1);
        assert_eq!(new_balance, initial_balance + chain_reward);

        System::assert_last_event(
            crate::Event::RewardClaimed {
                stash: STASH_1,
                amount: chain_reward,
            }
            .into(),
        );
    });
}

#[test]
fn accumulating_rewards_with_multiple_stashes_and_attestors_works() {
    ExtBuilder.build_and_execute(|| {
        let attestor = Attestor::new(STASH_1, ATTESTOR_1);

        assert_ok!(Attestation::register_attestor(
            attestor.stash.clone(),
            DEV_CHAIN_KEY,
            attestor.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor.attestor_id),
            DEV_CHAIN_KEY,
            attestor.public_key,
            attestor.signature
        ));

        let attestor2 = Attestor::new(STASH_1, ATTESTOR_2);

        assert_ok!(Attestation::register_attestor(
            attestor2.stash.clone(),
            DEV_CHAIN_KEY,
            attestor2.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor2.attestor_id),
            DEV_CHAIN_KEY,
            attestor2.public_key,
            attestor2.signature
        ));

        let attestor3 = Attestor::new(STASH_2, ATTESTOR_3);

        assert_ok!(Attestation::register_attestor(
            attestor3.stash.clone(),
            DEV_CHAIN_KEY,
            attestor3.attestor_id,
        ));

        // Toggle to active
        assert_ok!(Attestation::attest(
            RuntimeOrigin::signed(attestor3.attestor_id),
            DEV_CHAIN_KEY,
            attestor3.public_key,
            attestor3.signature
        ));

        progress_to_block(5);

        let attestation = create_signed_attestation(
            vec![attestor.clone(), attestor2.clone(), attestor3.clone()],
            1,
            0,
            None,
        );

        assert_ok!(Attestation::commit_attestation(
            RuntimeOrigin::none(),
            attestation.clone()
        ));

        // Get reward for chain 1
        let chain_reward = ChainReward::<Test>::get(1).unwrap();

        // Stash 1 has 2 attestors
        let rewards = AccumulatedRewards::<Test>::get(attestor.stash_id);
        assert!(rewards.is_some());
        let rewards = rewards.unwrap();
        assert_eq!(rewards, chain_reward * 2);

        // Stash 2 has 1 attestor
        let rewards = AccumulatedRewards::<Test>::get(attestor3.stash_id);
        assert!(rewards.is_some());
        let rewards = rewards.unwrap();
        assert_eq!(rewards, chain_reward);
    });
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
        attestors: attestors.iter().map(|a| a.attestor_id).collect::<Vec<_>>(),
    };

    attestation
}

#[test]
fn set_payee_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_payee(RuntimeOrigin::none(), RewardDestination::None),
            BadOrigin
        );
    })
}

#[test]
fn set_payee_should_error_when_signer_not_a_stash() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            Attestation::set_payee(RuntimeOrigin::signed(ATTESTOR_1), RewardDestination::None),
            Error::<Test>::NotStash
        );
    })
}

#[test]
fn set_payee_should_update_storage() {
    ExtBuilder.build_and_execute(|| {
        let payee = Payee::<Test>::get(STASH_1);
        assert!(payee.is_none());

        // setup
        let ledger: AttestorLedger<Test> = AttestorLedger::new(STASH_1, 100);
        Ledger::<Test>::insert(STASH_1, ledger);

        assert_ok!(Attestation::set_payee(
            RuntimeOrigin::signed(STASH_1),
            RewardDestination::None
        ),);

        let payee = Payee::<Test>::get(STASH_1).unwrap();
        assert_eq!(payee, RewardDestination::None);
    })
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
