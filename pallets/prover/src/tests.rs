use self::mock::PROVER_3;

use super::*;
use fp_account::AccountId20;
use frame_support::{assert_err, assert_ok};
use prover_primitives::claim::{Claim, ClaimKind};
use sp_runtime::traits::Hash;

use crate::mock::{ExtBuilder, ProverModule, RuntimeOrigin, Test, CLAIMER_1, PROVER_1};
use crate::types::{ChainPriceConfiguration, Prover};

fn prover_configured_in_genesis() -> RuntimeOrigin {
    RuntimeOrigin::signed(PROVER_3)
}

fn prover_1() -> RuntimeOrigin {
    RuntimeOrigin::signed(PROVER_1)
}

fn claimer_1() -> RuntimeOrigin {
    RuntimeOrigin::signed(CLAIMER_1)
}

#[test]
fn register_prover_should_work_happy_path() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));
    })
}

#[test]
fn register_prover_twice_should_fails() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));

        assert_err!(
            ProverModule::register_prover(prover_1(), Prover { nickname: vec![] }),
            Error::<Test>::ProverAlreadyExists
        );
    })
}

#[test]
fn add_chain_price_works() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));

        assert_ok!(ProverModule::set_chain_price_config(
            prover_1(),
            1,
            Some(ChainPriceConfiguration { price: 100 })
        ));
    })
}

#[test]
fn remove_chain_price_works() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));

        assert_ok!(ProverModule::set_chain_price_config(
            prover_1(),
            1,
            Some(ChainPriceConfiguration { price: 100 })
        ));

        assert_ok!(ProverModule::set_chain_price_config(prover_1(), 1, None));
    })
}

#[test]
fn create_claim_works() {
    ExtBuilder.build_and_execute(|| {
        let claim = Claim {
            block_number: 1,
            chain_id: 42,
            tx_index: 154,
            from: test_account_id20(),
            to: test_account_id20(),
            kind: ClaimKind::Tx,
        };

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim));
    })
}

#[test]
fn create_claim_twice_after_proof_submission_works() {
    ExtBuilder.build_and_execute(|| {
        let claim = Claim {
            block_number: 1,
            chain_id: 42,
            tx_index: 154,
            from: test_account_id20(),
            to: test_account_id20(),
            kind: ClaimKind::Tx,
        };

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim.clone(),));

        let claim_hash = <Test as Config>::Hashing::hash_of(&claim);

        assert_ok!(ProverModule::submit_proof(
            prover_configured_in_genesis(),
            claim_hash,
            b"some_proof".to_vec()
        ));

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim.clone(),));
    })
}

fn test_account_id20() -> AccountId20 {
    let hex_acc: [u8; 20] = hex::decode("98fa2838ee6471ae87135880f870a785318e6787")
        .unwrap()
        .try_into()
        .unwrap();

    AccountId20::from(hex_acc)
}
