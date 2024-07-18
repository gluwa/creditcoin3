use self::mock::PROVER_3;

use super::*;
use crate::types::{BlockItemIdentifier, Claim, ClaimId, ClaimKind};

use frame_support::{assert_err, assert_ok};
use sp_runtime::traits::Hash;

use crate::mock::{ExtBuilder, ProverModule, RuntimeOrigin, Test, CLAIMER_1, PROVER_1};
use crate::{types::Prover, ChainPriceConfiguration};

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
            vec![
                (ChainPriceConfiguration {
                    chain_id: 1,
                    price: 100
                })
            ]
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
            vec![
                (ChainPriceConfiguration {
                    chain_id: 1,
                    price: 100
                })
            ]
        ));

        assert_ok!(ProverModule::set_chain_price_config(prover_1(), vec![]));
    })
}

#[test]
fn create_claim_works() {
    ExtBuilder.build_and_execute(|| {
        let claim = Claim {
            chain_id: 1,
            id: ClaimId {
                kind: ClaimKind::Tx,
                block_item_id: BlockItemIdentifier {
                    block_number: 1,
                    index: 1,
                },
            },
            felt_ranges: vec![],
        };

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim));

        let locked_balance = ProverModule::get_locked_balance(&CLAIMER_1);
        assert_eq!(locked_balance, 100);
    })
}

#[test]
fn create_multiple_claims_works() {
    ExtBuilder.build_and_execute(|| {
        let claim = Claim {
            chain_id: 1,
            id: ClaimId {
                kind: ClaimKind::Tx,
                block_item_id: BlockItemIdentifier {
                    block_number: 1,
                    index: 1,
                },
            },
            felt_ranges: vec![],
        };

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim));

        let locked_balance = ProverModule::get_locked_balance(&CLAIMER_1);
        assert_eq!(locked_balance, 100);

        let claim = Claim {
            chain_id: 1,
            id: ClaimId {
                kind: ClaimKind::Tx,
                block_item_id: BlockItemIdentifier {
                    block_number: 1,
                    index: 1,
                },
            },
            felt_ranges: vec![],
        };

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim));

        let locked_balance = ProverModule::get_locked_balance(&CLAIMER_1);
        assert_eq!(locked_balance, 200);
    })
}

#[test]
fn submit_invalid_proof_fails() {
    ExtBuilder.build_and_execute(|| {
        let claim = Claim {
            chain_id: 1,
            id: ClaimId {
                kind: ClaimKind::Tx,
                block_item_id: BlockItemIdentifier {
                    block_number: 1,
                    index: 1,
                },
            },
            felt_ranges: vec![],
        };

        assert_ok!(ProverModule::submit_claim(claimer_1(), claim.clone(),));

        let claim_hash = <Test as Config>::Hashing::hash_of(&claim);

        assert_err!(
            ProverModule::submit_proof(
                prover_configured_in_genesis(),
                claim_hash,
                b"some_proof".to_vec()
            ),
            Error::<Test>::InvalidProofSubmitted
        );
    })
}

#[test]
fn submit_claim_for_unsupported_chain_fails() {
    ExtBuilder.build_and_execute(|| {
        // Setup prover and price
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));

        // None of the chains are supported
        let claim = Claim {
            chain_id: 2,
            id: ClaimId {
                kind: ClaimKind::Tx,
                block_item_id: BlockItemIdentifier {
                    block_number: 1,
                    index: 1,
                },
            },
            felt_ranges: vec![],
        };

        assert_err!(
            ProverModule::submit_claim(claimer_1(), claim.clone()),
            Error::<Test>::ChainNotSupported
        );
    })
}

#[test]
fn add_chain_price_for_unsupported_chain_fails() {
    ExtBuilder.build_and_execute(|| {
        // Setup prover and price
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));

        // None of the chains are supported
        assert_err!(
            ProverModule::set_chain_price_config(
                prover_1(),
                vec![
                    (ChainPriceConfiguration {
                        chain_id: 2,
                        price: 100
                    })
                ]
            ),
            Error::<Test>::ChainNotSupported
        );
    })
}
