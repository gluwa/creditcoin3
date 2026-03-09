use crate::{self as pallet, mock, mock::SupportedChain, mock::*, Error, SupportedChains};
use attestor_primitives::ChainEncodingVersion;
use frame_support::{assert_noop, assert_ok};
use rstest::rstest;
use sp_runtime::traits::BadOrigin;
use supported_chains_primitives::{
    provider::SupportedChainsProvider, MATURITY_EVM_FINALIZED, MATURITY_EVM_LATEST,
    MATURITY_EVM_SAFE, MATURITY_FIXED_DELAY, MATURITY_FIXED_DELAY_10,
};

#[test]
fn register_chain_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        // We don't use ExtBuilder from mock in this test. So the chain_id 200
        // hasn't already been registered to another chain at genesis.
        let chain_id = 200;
        let chain_name = "Ethereum".to_string();

        assert_eq!(SupportedChain::chain_key_value(), 1);
        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        ));
        assert_eq!(SupportedChain::chain_key_value(), 2);

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_some());
        assert_eq!(
            SupportedChains::<Test>::get(chain_key.expect("Should have a chain key")),
            Some(supported_chains_primitives::SupportedChain {
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
                chain_encoding: ChainEncodingVersion::V1,
                maturity_strategy: <mock::Test as pallet::Config>::DefaultMaturityStrategy::get()
                    .to_string()
            })
        );

        // assert on emited event
        System::assert_last_event(
            crate::Event::ChainRegistered {
                chain_key: chain_key.unwrap(),
                chain_id,
                chain_name: chain_name.into(),
                chain_encoding: ChainEncodingVersion::V1,
                maturity_strategy: <mock::Test as pallet::Config>::DefaultMaturityStrategy::get()
                    .to_string(),
            }
            .into(),
        );
    });
}

#[test]
fn register_chain_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_id = 201;
        let chain_name = "Ethereum".to_string();

        assert_noop!(
            SupportedChain::register_chain(
                RuntimeOrigin::none(),
                chain_id,
                chain_name,
                None,
                None,
                None,
                None,
                None,
                None,
                ChainEncodingVersion::V1,
                None,
            ),
            BadOrigin
        );
    });
}

#[test]
fn register_chain_should_error_when_not_signed_by_alice() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        // This should fail because account 2 is not ALICE (1)
        assert_noop!(
            SupportedChain::register_chain(
                RuntimeOrigin::signed(2),
                201,
                "Ethereum_1".to_owned(),
                None,
                None,
                None,
                None,
                None,
                None,
                ChainEncodingVersion::V1,
                None,
            ),
            BadOrigin
        );

        // This should succeed because ALICE is allowed in our MockOperators EnsureOrigin
        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::signed(ALICE),
            201,
            "Ethereum_1".to_owned(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        ));
    });
}

#[test]
fn register_chain_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_id = 201;
        let chain_name = "Ethereum".to_string();
        let acct: AccountId = 4;

        assert_noop!(
            SupportedChain::register_chain(
                RuntimeOrigin::signed(acct),
                chain_id,
                chain_name,
                None,
                None,
                None,
                None,
                None,
                None,
                ChainEncodingVersion::V1,
                None,
            ),
            BadOrigin
        );
    });
}

#[test]
fn register_chain_should_error_when_registering_duplicate_chain() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_id = 200; // id already included in storage
        let chain_name = "Ethereum".to_string(); // name already included in storage

        assert_noop!(
            SupportedChain::register_chain(
                RuntimeOrigin::root(),
                chain_id,
                chain_name,
                None,
                None,
                None,
                None,
                None,
                None,
                ChainEncodingVersion::V1,
                None,
            ),
            Error::<Test>::ChainAlreadyRegistered
        );
    });
}

#[test]
fn register_chain_should_work_when_registering_chain_with_duplicate_id_but_different_name() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_id = 200; // id already included in storage
        let chain_name = "Sepolia".to_string(); // name is different

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        ),);

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_some());
        assert_eq!(
            SupportedChains::<Test>::get(chain_key.expect("Should have a chain key")),
            Some(supported_chains_primitives::SupportedChain {
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
                chain_encoding: ChainEncodingVersion::V1,
                maturity_strategy: <mock::Test as pallet::Config>::DefaultMaturityStrategy::get()
                    .to_string()
            })
        );
    });
}

#[test]
fn register_chain_should_work_when_registering_chain_with_duplicate_name_but_different_id() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_id = 201; // id is different
        let chain_name = "Ethereum".to_string(); // name already included in storage

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        ),);

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_some());
        assert_eq!(
            SupportedChains::<Test>::get(chain_key.expect("Should have a chain key")),
            Some(supported_chains_primitives::SupportedChain {
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
                chain_encoding: ChainEncodingVersion::V1,
                maturity_strategy: <mock::Test as pallet::Config>::DefaultMaturityStrategy::get()
                    .to_string()
            })
        );
    });
}

#[test]
fn register_chain_should_error_when_chain_key_index_exceeded() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // we can store a maximum of u64::MAX chains in this pallet
        crate::ChainKeyValue::<Test>::put(u64::MAX);

        let chain_id = 33;
        let chain_name = "Ethereum".to_string();

        assert_noop!(
            SupportedChain::register_chain(
                RuntimeOrigin::root(),
                chain_id,
                chain_name,
                None,
                None,
                None,
                None,
                None,
                None,
                ChainEncodingVersion::V1,
                None,
            ),
            Error::<Test>::Arithmetic
        );
    });
}

#[test]
fn remove_chain_works() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_key = 1;

        // chain has already been added therefore index is now 2
        assert_eq!(SupportedChain::chain_key_value(), 2);
        assert_ok!(SupportedChain::remove_chain(
            RuntimeOrigin::root(),
            chain_key,
            false
        ));
        assert_eq!(SupportedChains::<Test>::get(chain_key), None);

        // internal index should not change
        assert_eq!(SupportedChain::chain_key_value(), 2);

        // assert on emited event
        System::assert_last_event(
            crate::Event::ChainRemoved {
                chain_key,
                chain_id: 200,
                chain_name: "Ethereum".into(),
                chain_encoding: ChainEncodingVersion::V1,
                maturity_strategy: <mock::Test as pallet::Config>::DefaultMaturityStrategy::get()
                    .to_string(),
            }
            .into(),
        );
    });
}

#[test]
fn remove_chain_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_key = 1;

        assert_noop!(
            SupportedChain::remove_chain(RuntimeOrigin::none(), chain_key, false),
            BadOrigin
        );
    });
}

#[test]
fn remove_chain_should_error_when_not_signed_by_alice() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_key = 1;

        // This should fail because account 2 is not ALICE (1)
        assert_noop!(
            SupportedChain::remove_chain(RuntimeOrigin::signed(2), chain_key, false),
            BadOrigin
        );

        // This should succeed because ALICE is allowed in our MockOperators EnsureOrigin
        assert_ok!(SupportedChain::remove_chain(
            RuntimeOrigin::signed(ALICE),
            chain_key,
            false
        ));
    });
}

#[test]
fn remove_chain_should_error_when_not_signed_by_root() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_key = 1;
        let acct: AccountId = 4;

        assert_noop!(
            SupportedChain::remove_chain(RuntimeOrigin::signed(acct), chain_key, false),
            BadOrigin
        );
    });
}

#[test]
fn remove_chain_should_error_when_chain_is_not_supported() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_key = 1;

        assert_noop!(
            SupportedChain::remove_chain(RuntimeOrigin::root(), chain_key, false),
            Error::<Test>::ChainNotSupported
        );
    });
}

#[test]
fn test_method_supported_chains() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 200;
        let chain_name = "Ethereum".to_string();

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        ));

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_some(), "Chain key should be present");

        let supported_chains = SupportedChain::supported_chains();
        assert_eq!(
            supported_chains,
            vec![chain_key.expect("Should have a chain key")]
        );
    });
}

#[test]
fn test_function_is_chain_supported() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = 1;

        let is_supported = SupportedChain::is_chain_supported(chain_key);
        assert!(is_supported);

        let bad_chain_key = 2;
        let is_supported = SupportedChain::is_chain_supported(bad_chain_key);
        assert!(!is_supported);
    });
}

#[test]
fn empty_supported_chains() {
    new_test_ext().execute_with(|| {
        let supported_chains = SupportedChain::supported_chains();
        assert!(supported_chains.is_empty());
    });
}

#[test]
#[should_panic]
fn build_should_panic_with_duplicate_chains_in_genesis() {
    ExtBuilder.build_and_execute_with_duplicate_chains(
        vec![
            (
                1,
                "Ethereum".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
            (
                1,
                "Ethereum".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
        ],
        || {
            System::set_block_number(1);
        },
    );
}

#[rstest]
#[case(MATURITY_EVM_FINALIZED.to_string())]
#[case(MATURITY_EVM_SAFE.to_string())]
#[case(MATURITY_EVM_LATEST.to_string())]
#[case(format!("{MATURITY_FIXED_DELAY}10"))]
#[case(format!("{MATURITY_FIXED_DELAY} 10"))]
fn register_chain_accepts_valid_maturity_strategy_and_stores_it(#[case] strategy: String) {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let chain_id = 200u64;
        let chain_name = "ethereum".to_string();

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            Some(strategy.clone()),
        ));

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_some());
        assert_eq!(
            SupportedChains::<Test>::get(chain_key.expect("Should have a chain key")),
            Some(supported_chains_primitives::SupportedChain {
                chain_id,
                chain_name: chain_name.as_bytes().to_vec(),
                chain_encoding: ChainEncodingVersion::V1,
                maturity_strategy: strategy,
            })
        );
    });
}

#[rstest]
#[case("".to_string())]
#[case("invalid".to_string())]
#[case(format!("{MATURITY_FIXED_DELAY}"))]
#[case(format!("{MATURITY_FIXED_DELAY}abc"))]
#[case(format!("{MATURITY_FIXED_DELAY}{MATURITY_FIXED_DELAY}10"))]
fn register_chain_rejects_invalid_maturity_strategy(#[case] strategy: String) {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);
        let chain_id = 200u64;
        let chain_name = "ethereum".to_string();

        assert_noop!(
            SupportedChain::register_chain(
                RuntimeOrigin::root(),
                chain_id,
                chain_name.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                ChainEncodingVersion::V1,
                Some(strategy),
            ),
            Error::<Test>::InvalidMaturityStrategy
        );

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_none());
    });
}
