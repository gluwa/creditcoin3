use super::*;
use frame_support::{assert_err, assert_ok};

use crate::mock::{ExtBuilder, ProverModule, RuntimeOrigin, Test, PROVER_1};
use crate::types::{ChainPriceConfiguration, Prover};

fn prover_1() -> RuntimeOrigin {
    RuntimeOrigin::signed(PROVER_1)
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
            ChainPriceConfiguration {
                chain_id: 1,
                price: 100
            }
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
            ChainPriceConfiguration {
                chain_id: 1,
                price: 100
            }
        ));

        assert_ok!(ProverModule::unset_chain_price_config(
            prover_1(),
            ChainPriceConfiguration {
                chain_id: 1,
                price: 100
            }
        ));
    })
}

#[test]
fn remove_non_existing_chain_price_fails() {
    ExtBuilder.build_and_execute(|| {
        assert_ok!(ProverModule::register_prover(
            prover_1(),
            Prover { nickname: vec![] }
        ));

        assert_err!(
            ProverModule::unset_chain_price_config(
                prover_1(),
                ChainPriceConfiguration {
                    chain_id: 1,
                    price: 100
                }
            ),
            Error::<Test>::ChainPriceConfigurationNotFound
        );
    })
}
