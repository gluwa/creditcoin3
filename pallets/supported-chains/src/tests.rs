use crate::{mock::SupportedChain, mock::*, Error, SupportedChains};
use frame_support::{assert_noop, assert_ok};
use supported_chains_primitives::provider::SupportedChainsProvider;

#[test]
fn register_chain_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_name = "Ethereum".to_string();

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone()
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
                chain_name: chain_name.as_bytes().to_vec()
            })
        );
    });
}

#[test]
fn register_duplicate_chain_returns_error() {
    ExtBuilder.build_and_execute(|| {
        System::set_block_number(1);
        let chain_id = 1;
        let chain_name = "Ethereum".to_string();

        assert_noop!(
            SupportedChain::register_chain(RuntimeOrigin::root(), chain_id, chain_name),
            Error::<Test>::ChainAlreadyRegistered
        );
    });
}

#[test]
fn remove_chain_works() {
    ExtBuilder.build_and_execute(|| {
        let chain_key = 1;

        assert_ok!(SupportedChain::remove_chain(
            RuntimeOrigin::root(),
            chain_key
        ));
        assert_eq!(SupportedChains::<Test>::get(chain_key), None);
    });
}

#[test]
fn remove_chain_returns_correct_error_when_chain_is_not_supported() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 1;

        assert_noop!(
            SupportedChain::remove_chain(RuntimeOrigin::root(), chain_id),
            Error::<Test>::ChainNotSupported
        );
    });
}

#[test]
fn test_method_supported_chains() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_name = "Ethereum".to_string();

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone()
        ));

        let chain_key = SupportedChain::chain_key_by_chain_id_and_name(
            chain_id,
            chain_name.as_bytes().to_vec(),
        );
        assert!(chain_key.is_some(), "Chain key should be present");

        let supported_chains = SupportedChain::supported_chains();
        assert_eq!(
            supported_chains,
            Some(vec![chain_key.expect("Should have a chain key")])
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
        assert_eq!(supported_chains, None);
    });
}
