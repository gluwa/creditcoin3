use crate::{
    self as pallet, mock, mock::SupportedChain, mock::*, CoreFees, Error, OutboxDiscoveries,
    OutboxFactories, SupportedChains, WriteAbilityConfigs,
};
use attestor_primitives::ChainEncodingVersion;
use frame_support::{assert_noop, assert_ok};
use rstest::rstest;
use sp_core::{H160, U256};
use sp_runtime::{traits::BadOrigin, BuildStorage};
use supported_chains_primitives::{
    provider::SupportedChainsProvider, MATURITY_EVM_FINALIZED, MATURITY_EVM_LATEST,
    MATURITY_EVM_SAFE, MATURITY_FIXED_DELAY, MATURITY_FIXED_DELAY_10, MATURITY_RPC_FINALIZED,
    MATURITY_RPC_SAFE,
};
use supported_chains_primitives::{CoreFeeConfig, WriteAbilityConfig};

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
fn register_chain_should_error_when_not_signed_by_operator() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        // This should fail because account 2 is not OPERATOR_ACCOUNT (1)
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

        // This should succeed because OPERATOR_ACCOUNT is allowed in our MockOperators EnsureOrigin
        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::signed(OPERATOR_ACCOUNT),
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
        let chain_key = SUPPORTED_CHAIN_KEY;

        // Seed every piece of per-chain state so one test proves removal clears all of it (folds the
        // former remove_chain_clears_write_ability_config / remove_chain_clears_core_fee tests and
        // adds the OutboxFactories/OutboxDiscoveries assertions).
        assert_ok!(SupportedChain::set_write_ability_config(
            RuntimeOrigin::root(),
            chain_key,
            [0x33u8; 32],
            true,
        ));
        assert_ok!(SupportedChain::set_core_fee(
            RuntimeOrigin::root(),
            chain_key,
            U256::from(7u64),
        ));
        assert_ok!(SupportedChain::set_outbox_factory_addr(
            RuntimeOrigin::root(),
            chain_key,
            H160::repeat_byte(0x55),
        ));
        assert_ok!(SupportedChain::set_outbox_discovery_addr(
            RuntimeOrigin::root(),
            chain_key,
            H160::repeat_byte(0x66),
        ));
        assert!(WriteAbilityConfigs::<Test>::get(chain_key).is_some());
        assert!(CoreFees::<Test>::get(chain_key).is_some());
        assert!(OutboxFactories::<Test>::get(chain_key).is_some());
        assert!(OutboxDiscoveries::<Test>::get(chain_key).is_some());

        // chain has already been added therefore index is now 2
        assert_eq!(SupportedChain::chain_key_value(), 2);
        assert_ok!(SupportedChain::remove_chain(
            RuntimeOrigin::root(),
            chain_key,
            false
        ));

        // The chain and every associated record are gone.
        assert_eq!(SupportedChains::<Test>::get(chain_key), None);
        assert_eq!(WriteAbilityConfigs::<Test>::get(chain_key), None);
        assert_eq!(CoreFees::<Test>::get(chain_key), None);
        assert_eq!(OutboxFactories::<Test>::get(chain_key), None);
        assert_eq!(OutboxDiscoveries::<Test>::get(chain_key), None);

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
fn remove_chain_should_error_when_not_signed_by_operator() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_key = 1;

        // This should fail because account 2 is not OPERATOR_ACCOUNT (1)
        assert_noop!(
            SupportedChain::remove_chain(RuntimeOrigin::signed(2), chain_key, false),
            BadOrigin
        );

        // This should succeed because OPERATOR_ACCOUNT is allowed in our MockOperators EnsureOrigin
        assert_ok!(SupportedChain::remove_chain(
            RuntimeOrigin::signed(OPERATOR_ACCOUNT),
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

#[test]
fn set_outbox_factory_addr_works() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_eq!(OutboxFactories::<Test>::get(chain_key), None);

        assert_ok!(SupportedChain::set_outbox_factory_addr(
            RuntimeOrigin::root(),
            chain_key,
            address,
        ));

        assert_eq!(OutboxFactories::<Test>::get(chain_key), Some(address));

        System::assert_last_event(
            crate::Event::OutboxFactoryRegistered {
                chain_key,
                outbox_factory_addr: address,
            }
            .into(),
        );
    });
}

#[test]
fn set_outbox_factory_addr_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_noop!(
            SupportedChain::set_outbox_factory_addr(RuntimeOrigin::none(), chain_key, address,),
            BadOrigin
        );
    });
}

#[test]
fn set_outbox_factory_addr_should_error_when_not_signed_by_operator() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_noop!(
            SupportedChain::set_outbox_factory_addr(RuntimeOrigin::signed(2), chain_key, address,),
            BadOrigin
        );

        assert_ok!(SupportedChain::set_outbox_factory_addr(
            RuntimeOrigin::signed(OPERATOR_ACCOUNT),
            chain_key,
            address,
        ));

        assert_eq!(OutboxFactories::<Test>::get(chain_key), Some(address));
    });
}

#[test]
fn set_outbox_factory_addr_should_error_when_chain_is_not_supported() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_noop!(
            SupportedChain::set_outbox_factory_addr(RuntimeOrigin::root(), chain_key, address,),
            Error::<Test>::ChainNotSupported
        );

        assert_eq!(OutboxFactories::<Test>::get(chain_key), None);
    });
}

#[test]
fn set_outbox_factory_addr_should_error_when_zero_address() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = 1;

        assert_noop!(
            SupportedChain::set_outbox_factory_addr(
                RuntimeOrigin::root(),
                chain_key,
                H160::zero(),
            ),
            Error::<Test>::ZeroOutboxFactoryAddress
        );

        assert_eq!(OutboxFactories::<Test>::get(chain_key), None);
    });
}

#[test]
fn set_outbox_discovery_addr_works() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_eq!(OutboxDiscoveries::<Test>::get(chain_key), None);

        assert_ok!(SupportedChain::set_outbox_discovery_addr(
            RuntimeOrigin::root(),
            chain_key,
            address,
        ));

        assert_eq!(OutboxDiscoveries::<Test>::get(chain_key), Some(address));

        System::assert_last_event(
            crate::Event::OutboxDiscoveryRegistered {
                chain_key,
                outbox_discovery_addr: address,
            }
            .into(),
        );
    });
}

#[test]
fn set_outbox_discovery_addr_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_noop!(
            SupportedChain::set_outbox_discovery_addr(RuntimeOrigin::none(), chain_key, address,),
            BadOrigin
        );
    });
}

#[test]
fn set_outbox_discovery_addr_should_error_when_not_signed_by_operator() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_noop!(
            SupportedChain::set_outbox_discovery_addr(
                RuntimeOrigin::signed(2),
                chain_key,
                address,
            ),
            BadOrigin
        );

        assert_ok!(SupportedChain::set_outbox_discovery_addr(
            RuntimeOrigin::signed(OPERATOR_ACCOUNT),
            chain_key,
            address,
        ));

        assert_eq!(OutboxDiscoveries::<Test>::get(chain_key), Some(address));
    });
}

#[test]
fn set_outbox_discovery_addr_should_error_when_chain_is_not_supported() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_key = 1;
        let address = H160::repeat_byte(0x11);

        assert_noop!(
            SupportedChain::set_outbox_discovery_addr(RuntimeOrigin::root(), chain_key, address,),
            Error::<Test>::ChainNotSupported
        );

        assert_eq!(OutboxDiscoveries::<Test>::get(chain_key), None);
    });
}

#[test]
fn set_outbox_discovery_addr_should_error_when_zero_address() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = 1;

        assert_noop!(
            SupportedChain::set_outbox_discovery_addr(
                RuntimeOrigin::root(),
                chain_key,
                H160::zero(),
            ),
            Error::<Test>::ZeroOutboxDiscoveryAddress
        );

        assert_eq!(OutboxDiscoveries::<Test>::get(chain_key), None);
    });
}

#[test]
fn set_write_ability_config_works() {
    // Parametrized over both accepted origins: root and an Operators member.
    for origin in [
        RuntimeOrigin::root(),
        RuntimeOrigin::signed(OPERATOR_ACCOUNT),
    ] {
        ExtBuilder.build_and_execute(|| {
            System::set_block_number(1);

            let chain_key = SUPPORTED_CHAIN_KEY;
            let write_ability_chain_key = [0x22u8; 32];

            assert_eq!(WriteAbilityConfigs::<Test>::get(chain_key), None);

            assert_ok!(SupportedChain::set_write_ability_config(
                origin,
                chain_key,
                write_ability_chain_key,
                true,
            ));

            assert_eq!(
                WriteAbilityConfigs::<Test>::get(chain_key),
                Some(WriteAbilityConfig {
                    write_ability_chain_key,
                    message_attestation_enabled: true,
                })
            );

            // Provider getter returns the stored config.
            assert_eq!(
                <SupportedChain as SupportedChainsProvider>::get_write_ability_config(chain_key),
                Some(WriteAbilityConfig {
                    write_ability_chain_key,
                    message_attestation_enabled: true,
                })
            );

            System::assert_last_event(
                crate::Event::WriteAbilityConfigSet {
                    chain_key,
                    write_ability_chain_key,
                    message_attestation_enabled: true,
                }
                .into(),
            );
        });
    }
}

#[test]
fn set_write_ability_config_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 1;

        assert_noop!(
            SupportedChain::set_write_ability_config(
                RuntimeOrigin::none(),
                chain_key,
                [0u8; 32],
                true,
            ),
            BadOrigin
        );
    });
}

#[test]
fn set_write_ability_config_should_error_when_not_signed_by_operator() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = SUPPORTED_CHAIN_KEY;

        assert_noop!(
            SupportedChain::set_write_ability_config(
                RuntimeOrigin::signed(2),
                chain_key,
                [0u8; 32],
                true,
            ),
            BadOrigin
        );
        // Operator-origin success is covered by set_write_ability_config_works (parametrized over
        // root + operator); this test only rejects a non-operator signer.
    });
}

#[test]
fn set_write_ability_config_should_error_when_chain_is_not_supported() {
    // `new_test_ext()` seeds no chains (unlike `ExtBuilder`, which registers one at
    // SUPPORTED_CHAIN_KEY), so this chain_key is deliberately unregistered here.
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_key = 1;

        assert_noop!(
            SupportedChain::set_write_ability_config(
                RuntimeOrigin::root(),
                chain_key,
                [0u8; 32],
                true,
            ),
            Error::<Test>::ChainNotSupported
        );

        assert_eq!(WriteAbilityConfigs::<Test>::get(chain_key), None);
    });
}

#[test]
fn set_write_ability_config_should_error_when_write_ability_key_is_zero() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        // Chain is supported (ExtBuilder seeds it), so the guard that fires is the zero
        // write-ability-key check — not ChainNotSupported. The zeroed argument is the
        // write_ability_chain_key ([0u8; 32]), not the chain_key.
        let chain_key = SUPPORTED_CHAIN_KEY;

        assert_noop!(
            SupportedChain::set_write_ability_config(
                RuntimeOrigin::root(),
                chain_key,
                [0u8; 32],
                true,
            ),
            Error::<Test>::ZeroWriteAbilityChainKey
        );

        assert_eq!(WriteAbilityConfigs::<Test>::get(chain_key), None);
    });
}

#[test]
fn genesis_seeds_write_ability_configs() {
    let mut storage = frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap();

    let write_ability_chain_key = [0x44u8; 32];
    let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
        supported_chains: vec![(
            200,
            "Ethereum".as_bytes().to_vec(),
            ChainEncodingVersion::V1,
            MATURITY_FIXED_DELAY_10.to_string(),
        )],
        write_ability_configs: vec![(1, write_ability_chain_key, true)],
        outbox_factories: Default::default(),
        outbox_discoveries: Default::default(),
        _phantom: Default::default(),
    };
    pallet_genesis.assimilate_storage(&mut storage).unwrap();

    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| {
        assert_eq!(
            WriteAbilityConfigs::<Test>::get(1),
            Some(WriteAbilityConfig {
                write_ability_chain_key,
                message_attestation_enabled: true,
            })
        );
    });
}

#[test]
fn genesis_seeds_outbox_factories() {
    let mut storage = frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap();

    let address = H160::repeat_byte(0x44);
    let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
        supported_chains: vec![(
            200,
            "Ethereum".as_bytes().to_vec(),
            ChainEncodingVersion::V1,
            MATURITY_FIXED_DELAY_10.to_string(),
        )],
        write_ability_configs: Default::default(),
        outbox_factories: vec![(1, address)],
        outbox_discoveries: Default::default(),
        _phantom: Default::default(),
    };
    pallet_genesis.assimilate_storage(&mut storage).unwrap();

    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| {
        assert_eq!(OutboxFactories::<Test>::get(1), Some(address));
        // The provider method backing the runtime API returns the same value.
        assert_eq!(
            <SupportedChain as SupportedChainsProvider>::get_outbox_factory_address(1),
            Some(address)
        );
    });
}

#[test]
fn genesis_seeds_outbox_discoveries() {
    let mut storage = frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap();

    let address = H160::repeat_byte(0x55);
    let pallet_genesis = crate::pallet::GenesisConfig::<Test> {
        supported_chains: vec![(
            200,
            "Ethereum".as_bytes().to_vec(),
            ChainEncodingVersion::V1,
            MATURITY_FIXED_DELAY_10.to_string(),
        )],
        write_ability_configs: Default::default(),
        outbox_factories: Default::default(),
        outbox_discoveries: vec![(1, address)],
        _phantom: Default::default(),
    };
    pallet_genesis.assimilate_storage(&mut storage).unwrap();

    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| {
        assert_eq!(OutboxDiscoveries::<Test>::get(1), Some(address));
        // The provider method backing the runtime API returns the same value.
        assert_eq!(
            <SupportedChain as SupportedChainsProvider>::get_outbox_discovery_address(1),
            Some(address)
        );
    });
}

#[test]
fn set_core_fee_works() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = SUPPORTED_CHAIN_KEY;
        let amount = U256::from(1_000_000_000_000_000_000u128); // 1 ATTEST

        assert_eq!(CoreFees::<Test>::get(chain_key), None);

        assert_ok!(SupportedChain::set_core_fee(
            RuntimeOrigin::root(),
            chain_key,
            amount,
        ));

        assert_eq!(
            CoreFees::<Test>::get(chain_key),
            Some(CoreFeeConfig { amount })
        );

        System::assert_last_event(crate::Event::CoreFeeSet { chain_key, amount }.into());
    });
}

#[test]
fn set_core_fee_overwrites() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);

        let chain_key = SUPPORTED_CHAIN_KEY;
        let amount = U256::from(5u64);

        assert_ok!(SupportedChain::set_core_fee(
            RuntimeOrigin::root(),
            chain_key,
            U256::from(1u64),
        ));
        // A governance fee change is a single call with no migration — the Outbox reads the new
        // value on its next publish.
        assert_ok!(SupportedChain::set_core_fee(
            RuntimeOrigin::root(),
            chain_key,
            amount,
        ));

        assert_eq!(
            CoreFees::<Test>::get(chain_key),
            Some(CoreFeeConfig { amount })
        );
    });
}

#[test]
fn set_core_fee_should_error_when_not_signed() {
    ExtBuilder.build_and_execute(|| {
        assert_noop!(
            SupportedChain::set_core_fee(RuntimeOrigin::none(), 1, U256::from(1u64)),
            BadOrigin
        );
    });
}

#[test]
fn set_core_fee_should_error_when_not_signed_by_operator() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = SUPPORTED_CHAIN_KEY;
        let amount = U256::from(1u64);

        assert_noop!(
            SupportedChain::set_core_fee(RuntimeOrigin::signed(2), chain_key, amount),
            BadOrigin
        );

        assert_ok!(SupportedChain::set_core_fee(
            RuntimeOrigin::signed(OPERATOR_ACCOUNT),
            chain_key,
            amount,
        ));

        assert_eq!(
            CoreFees::<Test>::get(chain_key),
            Some(CoreFeeConfig { amount })
        );
    });
}

#[test]
fn set_core_fee_should_error_when_chain_is_not_supported() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        assert_noop!(
            SupportedChain::set_core_fee(RuntimeOrigin::root(), 1, U256::from(1u64)),
            Error::<Test>::ChainNotSupported
        );

        assert_eq!(CoreFees::<Test>::get(1), None);
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
            RuntimeOrigin::signed(OPERATOR_ACCOUNT),
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
