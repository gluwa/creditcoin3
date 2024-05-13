use crate::{mock::SupportedChain, mock::*, Error, SupportedChains};
use frame_support::{assert_noop, assert_ok};

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
        assert_eq!(
            SupportedChains::<Test>::get(chain_id),
            Some(chain_name.as_bytes().to_vec())
        );
    });
}

#[test]
fn register_duplicate_chain_returns_error() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_name = "Ethereum".to_string();

        let _ = SupportedChain::register_chain(RuntimeOrigin::root(), chain_id, chain_name.clone());

        assert_noop!(
            SupportedChain::register_chain(RuntimeOrigin::root(), chain_id, chain_name),
            Error::<Test>::ChainAlreadyRegistered
        );
    });
}

#[test]
fn remove_chain_works() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_name = "Ethereum".to_string();

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone()
        ));
        assert_eq!(
            SupportedChains::<Test>::get(chain_id),
            Some(chain_name.clone().as_bytes().to_vec())
        );

        assert_ok!(SupportedChain::remove_chain(
            RuntimeOrigin::root(),
            chain_id
        ));
        assert_eq!(SupportedChains::<Test>::get(chain_id), None);
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

        let supported_chains = SupportedChain::supported_chains();
        assert_eq!(supported_chains, Some(vec![chain_id]));
    });
}

#[test]
fn test_function_is_chain_supported() {
    new_test_ext().execute_with(|| {
        System::set_block_number(1);

        let chain_id = 1;
        let chain_name = "Ethereum".to_string();

        assert_ok!(SupportedChain::register_chain(
            RuntimeOrigin::root(),
            chain_id,
            chain_name.clone()
        ));

        let is_supported = SupportedChain::is_chain_supported(chain_id);
        assert!(is_supported);

        let bad_chain_id = 2;
        let is_supported = SupportedChain::is_chain_supported(bad_chain_id);
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
