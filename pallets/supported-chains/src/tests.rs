use crate::{self as pallet, mock, mock::SupportedChain, mock::*, Error, SupportedChains};
use attestor_primitives::ChainEncodingVersion;
use frame_support::{assert_noop, assert_ok};
use rstest::rstest;
use sp_runtime::traits::BadOrigin;
use supported_chains_primitives::{
    provider::SupportedChainsProvider, MATURITY_EVM_FINALIZED, MATURITY_EVM_LATEST,
    MATURITY_EVM_SAFE, MATURITY_FIXED_DELAY, MATURITY_FIXED_DELAY_10, MATURITY_RPC_FINALIZED,
    MATURITY_RPC_SAFE,
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
#[case(MATURITY_RPC_SAFE.to_string())]
#[case(MATURITY_RPC_FINALIZED.to_string())]
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
#[case("rpcsafe".to_string())]
#[case("RpcLatest".to_string())]
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

// ------------------------------- [ set_maturity_strategy ] ---------------------------------- //
//
// The genesis chain from `ExtBuilder` is chain key 1 (chain id 200, "Ethereum") registered with
// `MATURITY_FIXED_DELAY_10`.

const TEST_CHAIN_KEY: u64 = 1;
const TEST_CHAIN_ID: u64 = 200;

#[rstest]
#[case(MATURITY_EVM_FINALIZED.to_string())]
#[case(MATURITY_EVM_SAFE.to_string())]
#[case(MATURITY_EVM_LATEST.to_string())]
#[case(MATURITY_RPC_SAFE.to_string())]
#[case(MATURITY_RPC_FINALIZED.to_string())]
#[case(format!("{MATURITY_FIXED_DELAY}25"))]
#[case(format!("{MATURITY_FIXED_DELAY} 25"))]
fn set_maturity_strategy_replaces_the_stored_strategy_and_emits(#[case] strategy: String) {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(SupportedChain::set_maturity_strategy(
            RuntimeOrigin::root(),
            TEST_CHAIN_KEY,
            strategy.clone(),
        ));

        let stored = SupportedChains::<Test>::get(TEST_CHAIN_KEY).expect("chain is registered");
        assert_eq!(stored.maturity_strategy, strategy);
        // Nothing else about the registration may move.
        assert_eq!(stored.chain_id, TEST_CHAIN_ID);
        assert_eq!(stored.chain_name, "Ethereum".as_bytes().to_vec());
        assert_eq!(stored.chain_encoding, ChainEncodingVersion::V1);

        System::assert_last_event(
            crate::Event::MaturityStrategySet {
                chain_key: TEST_CHAIN_KEY,
                chain_id: TEST_CHAIN_ID,
                maturity_strategy: strategy,
            }
            .into(),
        );
    });
}

#[test]
fn set_maturity_strategy_works_for_an_operator() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(SupportedChain::set_maturity_strategy(
            RuntimeOrigin::signed(ALICE),
            TEST_CHAIN_KEY,
            MATURITY_EVM_FINALIZED.to_string(),
        ));
        assert_eq!(
            SupportedChains::<Test>::get(TEST_CHAIN_KEY)
                .expect("chain is registered")
                .maturity_strategy,
            MATURITY_EVM_FINALIZED
        );
    });
}

#[rstest]
#[case(RuntimeOrigin::none())]
#[case(RuntimeOrigin::signed(4))]
fn set_maturity_strategy_rejects_an_unprivileged_origin(#[case] origin: RuntimeOrigin) {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_maturity_strategy(
                origin,
                TEST_CHAIN_KEY,
                MATURITY_EVM_FINALIZED.to_string(),
            ),
            BadOrigin
        );
    });
}

#[test]
fn set_maturity_strategy_rejects_an_unregistered_chain() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_maturity_strategy(
                RuntimeOrigin::root(),
                TEST_CHAIN_KEY + 41,
                MATURITY_EVM_FINALIZED.to_string(),
            ),
            Error::<Test>::ChainNotSupported
        );
    });
}

#[rstest]
#[case("".to_string())]
#[case("invalid".to_string())]
#[case(format!("{MATURITY_FIXED_DELAY}"))]
#[case(format!("{MATURITY_FIXED_DELAY}abc"))]
#[case("rpcsafe".to_string())]
#[case("RpcLatest".to_string())]
#[case(format!("{MATURITY_FIXED_DELAY}{MATURITY_FIXED_DELAY}10"))]
fn set_maturity_strategy_rejects_an_invalid_strategy(#[case] strategy: String) {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_maturity_strategy(RuntimeOrigin::root(), TEST_CHAIN_KEY, strategy,),
            Error::<Test>::InvalidMaturityStrategy
        );
        // The registration is untouched.
        assert_eq!(
            SupportedChains::<Test>::get(TEST_CHAIN_KEY)
                .expect("chain is registered")
                .maturity_strategy,
            MATURITY_FIXED_DELAY_10
        );
    });
}

/// Re-submitting the strategy the chain already has is rejected rather than applied, so that
/// every `MaturityStrategySet` event stands for a real change: the event is what tells operators
/// the chain's attestors and archivers are due a restart.
#[test]
fn set_maturity_strategy_rejects_the_current_strategy() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_maturity_strategy(
                RuntimeOrigin::root(),
                TEST_CHAIN_KEY,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
            Error::<Test>::MaturityStrategyUnchanged
        );
    });
}

/// `FixedDelay: 10` and `FixedDelay:10` parse to the same strategy but are different strings.
/// The unchanged-check is on the stored string, so the spelling change is accepted — harmless,
/// and cheaper than teaching the pallet to parse for an equality test it does not otherwise need.
#[test]
fn set_maturity_strategy_treats_a_respelled_fixed_delay_as_a_change() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let respelled = format!("{MATURITY_FIXED_DELAY}10");
        assert_ne!(respelled, MATURITY_FIXED_DELAY_10);

        assert_ok!(SupportedChain::set_maturity_strategy(
            RuntimeOrigin::root(),
            TEST_CHAIN_KEY,
            respelled.clone(),
        ));
        assert_eq!(
            SupportedChains::<Test>::get(TEST_CHAIN_KEY)
                .expect("chain is registered")
                .maturity_strategy,
            respelled
        );
    });
}

/// A removed chain cannot have its strategy set, and setting it on a live chain does not disturb
/// its sibling registrations.
#[test]
fn set_maturity_strategy_is_scoped_to_one_chain() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            201,
            "Sepolia".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            Some(MATURITY_EVM_SAFE.to_string()),
        ));
        let other_key =
            SupportedChain::chain_key_by_chain_id_and_name(201, "Sepolia".as_bytes().to_vec())
                .expect("just registered");

        assert_ok!(SupportedChain::set_maturity_strategy(
            RuntimeOrigin::root(),
            TEST_CHAIN_KEY,
            MATURITY_RPC_FINALIZED.to_string(),
        ));

        assert_eq!(
            SupportedChains::<Test>::get(other_key)
                .expect("chain is registered")
                .maturity_strategy,
            MATURITY_EVM_SAFE
        );
    });
}

// ---------------------------------- [ set_chain_name ] ------------------------------------- //

#[test]
fn set_chain_name_replaces_the_stored_name_moves_the_lookup_and_emits() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let old_name = "Ethereum".as_bytes().to_vec();
        let new_name = "Ethereum Mainnet".as_bytes().to_vec();

        assert_ok!(SupportedChain::set_chain_name(
            RuntimeOrigin::root(),
            TEST_CHAIN_KEY,
            "Ethereum Mainnet".to_string(),
        ));

        let stored = SupportedChains::<Test>::get(TEST_CHAIN_KEY).expect("chain is registered");
        assert_eq!(stored.chain_name, new_name);
        // Nothing else about the registration may move.
        assert_eq!(stored.chain_id, TEST_CHAIN_ID);
        assert_eq!(stored.chain_encoding, ChainEncodingVersion::V1);
        assert_eq!(stored.maturity_strategy, MATURITY_FIXED_DELAY_10);

        // The (chain_id, name) lookup follows the rename: the old name no longer resolves and
        // the new one resolves to the same key.
        assert_eq!(
            pallet::ChainIdAndNameToUniqKey::<Test>::get(TEST_CHAIN_ID, &old_name),
            None
        );
        assert_eq!(
            pallet::ChainIdAndNameToUniqKey::<Test>::get(TEST_CHAIN_ID, &new_name),
            Some(TEST_CHAIN_KEY)
        );

        System::assert_last_event(
            crate::Event::ChainNameSet {
                chain_key: TEST_CHAIN_KEY,
                chain_id: TEST_CHAIN_ID,
                old_chain_name: old_name,
                chain_name: new_name,
            }
            .into(),
        );
    });
}

#[test]
fn set_chain_name_frees_the_old_name_for_a_new_registration() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(SupportedChain::set_chain_name(
            RuntimeOrigin::root(),
            TEST_CHAIN_KEY,
            "Ethereum Mainnet".to_string(),
        ));

        // The vacated (chain_id, "Ethereum") pair can be registered again as a fresh chain.
        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            TEST_CHAIN_ID,
            "Ethereum".to_string(),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        ));
        assert_eq!(
            pallet::ChainIdAndNameToUniqKey::<Test>::get(TEST_CHAIN_ID, "Ethereum".as_bytes()),
            Some(TEST_CHAIN_KEY + 1)
        );
    });
}

#[test]
fn set_chain_name_works_for_an_operator() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_ok!(SupportedChain::set_chain_name(
            RuntimeOrigin::signed(ALICE),
            TEST_CHAIN_KEY,
            "Ethereum Mainnet".to_string(),
        ));
        assert_eq!(
            SupportedChains::<Test>::get(TEST_CHAIN_KEY)
                .expect("chain is registered")
                .chain_name,
            "Ethereum Mainnet".as_bytes().to_vec()
        );
    });
}

#[rstest]
#[case(RuntimeOrigin::none())]
#[case(RuntimeOrigin::signed(4))]
fn set_chain_name_rejects_an_unprivileged_origin(#[case] origin: RuntimeOrigin) {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_chain_name(origin, TEST_CHAIN_KEY, "Ethereum Mainnet".to_string()),
            BadOrigin
        );
    });
}

#[test]
fn set_chain_name_rejects_an_unregistered_chain() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_chain_name(
                RuntimeOrigin::root(),
                TEST_CHAIN_KEY + 41,
                "Ethereum Mainnet".to_string(),
            ),
            Error::<Test>::ChainNotSupported
        );
    });
}

#[test]
fn set_chain_name_rejects_the_current_name() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_chain_name(
                RuntimeOrigin::root(),
                TEST_CHAIN_KEY,
                "Ethereum".to_string()
            ),
            Error::<Test>::ChainNameUnchanged
        );
    });
}

#[test]
fn set_chain_name_rejects_a_name_already_used_for_the_same_chain_id() {
    // Two chains share chain id 200 under different names; renaming one onto the other's
    // name would make the (chain_id, name) lookup ambiguous.
    ExtBuilder.build_and_execute_with_duplicate_chains(
        vec![
            (
                TEST_CHAIN_ID,
                "Ethereum".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
            (
                TEST_CHAIN_ID,
                "Ethereum Mainnet".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
        ],
        || {
            System::set_block_number(1);

            assert_noop!(
                SupportedChain::set_chain_name(
                    RuntimeOrigin::root(),
                    TEST_CHAIN_KEY,
                    "Ethereum Mainnet".to_string(),
                ),
                Error::<Test>::ChainAlreadyRegistered
            );
            // Both registrations are untouched.
            assert_eq!(
                pallet::ChainIdAndNameToUniqKey::<Test>::get(TEST_CHAIN_ID, "Ethereum".as_bytes()),
                Some(TEST_CHAIN_KEY)
            );
            assert_eq!(
                pallet::ChainIdAndNameToUniqKey::<Test>::get(
                    TEST_CHAIN_ID,
                    "Ethereum Mainnet".as_bytes()
                ),
                Some(TEST_CHAIN_KEY + 1)
            );
        },
    );
}

#[test]
fn set_chain_name_allows_a_name_used_under_another_chain_id() {
    // Names only need to be unique per chain id, same as on registration.
    ExtBuilder.build_and_execute_with_duplicate_chains(
        vec![
            (
                TEST_CHAIN_ID,
                "Ethereum".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
            (
                TEST_CHAIN_ID + 1,
                "Sepolia".as_bytes().to_vec(),
                ChainEncodingVersion::V1,
                MATURITY_FIXED_DELAY_10.to_string(),
            ),
        ],
        || {
            System::set_block_number(1);

            assert_ok!(SupportedChain::set_chain_name(
                RuntimeOrigin::root(),
                TEST_CHAIN_KEY,
                "Sepolia".to_string(),
            ));
            assert_eq!(
                pallet::ChainIdAndNameToUniqKey::<Test>::get(TEST_CHAIN_ID, "Sepolia".as_bytes()),
                Some(TEST_CHAIN_KEY)
            );
            assert_eq!(
                pallet::ChainIdAndNameToUniqKey::<Test>::get(
                    TEST_CHAIN_ID + 1,
                    "Sepolia".as_bytes()
                ),
                Some(TEST_CHAIN_KEY + 1)
            );
        },
    );
}

mod max_supported_chains {
    use super::*;
    use crate::migrations::MigrateSupportedChainsCountV0ToV1;
    use frame_support::traits::{Get, GetStorageVersion, OnRuntimeUpgrade, StorageVersion};

    fn register(chain_id: u64) -> frame_support::dispatch::DispatchResult {
        SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            format!("Chain {chain_id}"),
            None,
            None,
            None,
            None,
            None,
            None,
            ChainEncodingVersion::V1,
            None,
        )
    }

    fn max() -> u32 {
        <Test as pallet::Config>::MaxSupportedChains::get()
    }

    #[test]
    fn genesis_seeds_the_count() {
        ExtBuilder.build_and_execute(|| {
            assert_eq!(SupportedChain::supported_chains_count(), 1);
        });
    }

    #[test]
    fn register_chain_rejects_a_chain_past_the_cap() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            // Genesis already registered one chain.
            for chain_id in 1..u64::from(max()) {
                assert_ok!(register(chain_id));
            }
            assert_eq!(SupportedChain::supported_chains_count(), max());
            assert_eq!(SupportedChain::supported_chains().len() as u32, max());

            let key_before = SupportedChain::chain_key_value();
            assert_noop!(register(1_000), Error::<Test>::TooManyChains);
            assert_eq!(SupportedChain::chain_key_value(), key_before);
            assert_eq!(SupportedChain::supported_chains_count(), max());
        });
    }

    #[test]
    fn remove_chain_frees_a_slot() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);
            for chain_id in 1..u64::from(max()) {
                assert_ok!(register(chain_id));
            }
            assert_noop!(register(1_000), Error::<Test>::TooManyChains);

            assert_ok!(SupportedChain::remove_chain(
                RuntimeOrigin::root(),
                1,
                false
            ));
            assert_eq!(SupportedChain::supported_chains_count(), max() - 1);

            assert_ok!(register(1_000));
            assert_eq!(SupportedChain::supported_chains_count(), max());
            assert_noop!(register(1_001), Error::<Test>::TooManyChains);
        });
    }

    #[test]
    fn failed_removal_leaves_the_count_untouched() {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);
            assert_noop!(
                SupportedChain::remove_chain(RuntimeOrigin::root(), 99, false),
                Error::<Test>::ChainNotSupported
            );
            assert_eq!(SupportedChain::supported_chains_count(), 1);
        });
    }

    #[test]
    #[should_panic(expected = "more chains than `MaxSupportedChains`")]
    fn genesis_panics_when_it_registers_more_chains_than_the_cap() {
        let chains = (0..=u64::from(max()))
            .map(|id| {
                (
                    id,
                    format!("Chain {id}").into_bytes(),
                    ChainEncodingVersion::V1,
                    MATURITY_FIXED_DELAY_10.to_string(),
                )
            })
            .collect();
        ExtBuilder.build_and_execute_with_duplicate_chains(chains, || {});
    }

    #[test]
    fn migration_counts_the_existing_chains_and_bumps_the_version() {
        new_test_ext().execute_with(|| {
            // Pre-upgrade state: chains exist, no counter, storage version 0.
            for key in 1..=3u64 {
                SupportedChains::<Test>::insert(
                    key,
                    supported_chains_primitives::SupportedChain {
                        chain_id: key,
                        chain_name: format!("Chain {key}").into_bytes(),
                        chain_encoding: ChainEncodingVersion::V1,
                        maturity_strategy: MATURITY_FIXED_DELAY_10.to_string(),
                    },
                );
            }
            StorageVersion::new(0).put::<SupportedChain>();
            assert_eq!(SupportedChain::supported_chains_count(), 0);

            MigrateSupportedChainsCountV0ToV1::<Test>::on_runtime_upgrade();

            assert_eq!(SupportedChain::supported_chains_count(), 3);
            assert_eq!(
                SupportedChain::on_chain_storage_version(),
                StorageVersion::new(1)
            );
        });
    }

    #[test]
    fn migration_is_a_no_op_once_applied() {
        new_test_ext().execute_with(|| {
            StorageVersion::new(1).put::<SupportedChain>();
            crate::SupportedChainsCount::<Test>::put(7);

            MigrateSupportedChainsCountV0ToV1::<Test>::on_runtime_upgrade();

            assert_eq!(SupportedChain::supported_chains_count(), 7);
        });
    }
}
