//! Attest-coin upgrade and EVM boundary regressions using the production runtime/runner.

use super::*;
use frame_support::{assert_ok, traits::OnRuntimeUpgrade};
use pallet_evm::{AddressMapping as _, CallInfo, GasWeightMapping, Runner as _};
use sp_runtime::{traits::StaticLookup, BuildStorage};

const GAS_LIMIT: u64 = 3_000_000;

fn sender() -> H160 {
    H160::repeat_byte(0xAA)
}

fn sender_account() -> AccountId {
    AddressMapping::into_account_id(sender())
}

fn ext() -> sp_io::TestExternalities {
    let mut storage = frame_system::GenesisConfig::<Runtime>::default()
        .build_storage()
        .unwrap();
    pallet_balances::GenesisConfig::<Runtime> {
        balances: vec![(sender_account(), 100 * CTC)],
        ..Default::default()
    }
    .assimilate_storage(&mut storage)
    .unwrap();
    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| System::set_block_number(1));
    ext
}

fn migrate() {
    migrations::EnsureAttestCoinAssetRoles::<Runtime>::on_runtime_upgrade();
}

fn abi(signature: &[u8], words: &[U256]) -> Vec<u8> {
    let mut input = sp_io::hashing::keccak_256(signature)[..4].to_vec();
    for word in words {
        input.extend_from_slice(&word.to_big_endian());
    }
    input
}

fn address_word(address: H160) -> U256 {
    U256::from_big_endian(address.as_bytes())
}

fn call_with_proof_limit(target: H160, input: Vec<u8>, value: U256, proof: u64) -> CallInfo {
    let mut weight =
        <Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(GAS_LIMIT, true);
    *weight.proof_size_mut() = proof;
    <Runtime as pallet_evm::Config>::Runner::call(
        sender(),
        target,
        input,
        value,
        GAS_LIMIT,
        None,
        None,
        None,
        vec![],
        vec![],
        false,
        false,
        Some(weight),
        Some(0),
        <Runtime as pallet_evm::Config>::config(),
    )
    .expect("runner accepts the call")
}

fn call(target: H160, input: Vec<u8>) -> CallInfo {
    call_with_proof_limit(
        target,
        input,
        U256::zero(),
        GAS_LIMIT / GasLimitPovSizeRatio::get(),
    )
}

fn successful_call(target: H160, input: Vec<u8>) -> CallInfo {
    let info = call(target, input);
    assert!(info.exit_reason.is_succeed(), "{info:?}");
    info
}

/// Use the same compiled ERC-20 as the blockchain integration tests, including its real storage
/// writes and logs. Mocks of PrecompileHandle cannot establish cross-frame EVM rollback.
fn deploy_token() -> H160 {
    let artifact: serde_json::Value = serde_json::from_str(include_str!(
        "../../cli/src/test/blockchain-tests/artifacts/MockAttestToken.json"
    ))
    .unwrap();
    let hex = artifact["bytecode"]
        .as_str()
        .unwrap()
        .trim_start_matches("0x");
    let bytecode = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect();
    let info = <Runtime as pallet_evm::Config>::Runner::create(
        sender(),
        bytecode,
        U256::zero(),
        GAS_LIMIT,
        None,
        None,
        None,
        vec![],
        vec![],
        false,
        false,
        Some(<Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(GAS_LIMIT, true)),
        Some(0),
        <Runtime as pallet_evm::Config>::config(),
    )
    .unwrap();
    assert!(info.exit_reason.is_succeed(), "{info:?}");
    pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(info.value);
    info.value
}

fn token_balance(token: H160, holder: H160) -> U256 {
    U256::from_big_endian(
        &successful_call(token, abi(b"balanceOf(address)", &[address_word(holder)])).value,
    )
}

#[test]
fn permissionless_asset_accounts_reserve_a_deposit_per_entry() {
    ext().execute_with(|| {
        migrate();
        let deposit = AssetAccountDeposit::get();
        assert!(deposit > 0);
        let before = Balances::reserved_balance(sender_account());
        for prefix in 1u8..=3 {
            let beneficiary = AccountId::from([prefix; 32]);
            assert_ok!(Assets::touch_other(
                RuntimeOrigin::signed(sender_account()),
                ATTEST_COIN_ASSET_ID,
                NativeOrEvmAddressLookup::unlookup(beneficiary.clone()),
            ));
            assert_eq!(
                Balances::reserved_balance(sender_account()),
                before + u128::from(prefix) * deposit
            );
            assert!(!frame_system::Account::<Runtime>::contains_key(
                &beneficiary
            ));
        }
        let unfunded = AccountId::from([0xBB; 32]);
        let target = AccountId::from([0xCC; 32]);
        assert!(Assets::touch_other(
            RuntimeOrigin::signed(unfunded),
            ATTEST_COIN_ASSET_ID,
            NativeOrEvmAddressLookup::unlookup(target.clone()),
        )
        .is_err());
        assert!(!pallet_assets::Account::<Runtime>::contains_key(
            ATTEST_COIN_ASSET_ID,
            target
        ));
    });
}

#[test]
fn upgrade_creates_metadata_and_sets_roles_idempotently() {
    ext().execute_with(|| {
        pallet_sudo::Key::<Runtime>::put(sender_account());
        for _ in 0..2 {
            migrate();
            let details = pallet_assets::Asset::<Runtime>::get(ATTEST_COIN_ASSET_ID).unwrap();
            assert_eq!(details.owner, sender_account());
            assert_eq!(details.freezer, sender_account());
            assert_eq!(details.issuer, attest_coin_precompile_account());
            assert_eq!(details.admin, attest_coin_precompile_account());
            let metadata = pallet_assets::Metadata::<Runtime>::get(ATTEST_COIN_ASSET_ID);
            assert_eq!(metadata.decimals, 18);
            assert_eq!(metadata.name.as_slice(), b"Attest Coin");
            assert_eq!(metadata.symbol.as_slice(), b"AC");
        }
    });
}

#[test]
fn upgrade_repairs_existing_asset_roles_and_preserves_governance_metadata() {
    ext().execute_with(|| {
        let owner = AccountId::from([0x01; 32]);
        let freezer = AccountId::from([0x02; 32]);
        assert_ok!(Assets::force_create(
            RuntimeOrigin::root(),
            ATTEST_COIN_ASSET_ID,
            NativeOrEvmAddressLookup::unlookup(owner.clone()),
            false,
            1
        ));
        assert_ok!(Assets::set_team(
            RuntimeOrigin::signed(owner.clone()),
            ATTEST_COIN_ASSET_ID,
            NativeOrEvmAddressLookup::unlookup(owner.clone()),
            NativeOrEvmAddressLookup::unlookup(owner.clone()),
            NativeOrEvmAddressLookup::unlookup(freezer.clone())
        ));
        assert_ok!(Assets::force_set_metadata(
            RuntimeOrigin::root(),
            ATTEST_COIN_ASSET_ID,
            b"Custom name".to_vec(),
            b"CUSTOM".to_vec(),
            18,
            true
        ));
        let before = pallet_assets::Metadata::<Runtime>::get(ATTEST_COIN_ASSET_ID);
        migrate();
        let details = pallet_assets::Asset::<Runtime>::get(ATTEST_COIN_ASSET_ID).unwrap();
        // There is no sudo key: preserve the existing split owner/freezer.
        assert_eq!(details.owner, owner);
        assert_eq!(details.freezer, freezer);
        assert_eq!(details.issuer, attest_coin_precompile_account());
        assert_eq!(details.admin, attest_coin_precompile_account());
        assert_eq!(
            pallet_assets::Metadata::<Runtime>::get(ATTEST_COIN_ASSET_ID),
            before
        );
    });
}

#[test]
fn upgrade_initializes_missing_metadata_even_when_roles_already_match() {
    ext().execute_with(|| {
        assert_ok!(Assets::force_create(
            RuntimeOrigin::root(),
            ATTEST_COIN_ASSET_ID,
            NativeOrEvmAddressLookup::unlookup(attest_coin_precompile_account()),
            false,
            1
        ));
        migrate();
        assert_eq!(
            pallet_assets::Metadata::<Runtime>::get(ATTEST_COIN_ASSET_ID).decimals,
            18
        );
    });
}

#[test]
fn nonpayable_getter_reverts_native_value_transfer() {
    ext().execute_with(|| {
        let target = attest_coin_precompile_h160();
        let before = Balances::free_balance(attest_coin_precompile_account());
        let info = call_with_proof_limit(
            target,
            abi(b"accrued(bytes32)", &[U256::from(42)]),
            U256::from(CTC),
            GAS_LIMIT / GasLimitPovSizeRatio::get(),
        );
        assert!(info.exit_reason.is_revert(), "{info:?}");
        assert_eq!(
            Balances::free_balance(attest_coin_precompile_account()),
            before
        );
        assert_eq!(Balances::free_balance(sender_account()), 100 * CTC);
    });
}

#[test]
fn native_reward_read_requires_proof_budget_beyond_wrapper_checks() {
    ext().execute_with(|| {
        let stash = AccountId::from([0x42; 32]);
        pallet_attest_coin_rewards::Accrued::<Runtime>::insert(&stash, 12345u128);
        let target = attest_coin_precompile_h160();
        let wrapper = call(target, vec![0xFF; 4]);
        let wrapper_proof = wrapper.weight_info.unwrap().proof_size_usage.unwrap();
        let input = abi(b"accrued(bytes32)", &[U256::from_big_endian(&[0x42; 32])]);
        let limited = call_with_proof_limit(target, input.clone(), U256::zero(), wrapper_proof);
        assert!(limited.exit_reason.is_error(), "{limited:?}");
        let enough = successful_call(target, input);
        assert_eq!(U256::from_big_endian(&enough.value), U256::from(12345u128));
        assert!(enough.weight_info.unwrap().proof_size_usage.unwrap() > wrapper_proof);
    });
}

#[test]
fn metered_bridge_round_trip_creates_and_removes_asset_account_without_touch_deposit() {
    ext().execute_with(|| {
        migrate();
        let token = deploy_token();
        let bridge = attest_coin_precompile_h160();
        successful_call(
            token,
            abi(
                b"mint(address,uint256)",
                &[address_word(sender()), U256::from(100)],
            ),
        );
        successful_call(
            token,
            abi(
                b"approve(address,uint256)",
                &[address_word(bridge), U256::from(100)],
            ),
        );
        assert!(!pallet_assets::Account::<Runtime>::contains_key(
            ATTEST_COIN_ASSET_ID,
            sender_account()
        ));
        let reserved = Balances::reserved_balance(sender_account());
        let info = successful_call(bridge, abi(b"deposit(uint256)", &[U256::from(100)]));
        assert!(info.weight_info.unwrap().proof_size_usage.unwrap() > 3675);
        assert_eq!(Assets::balance(ATTEST_COIN_ASSET_ID, sender_account()), 100);
        assert_eq!(Balances::reserved_balance(sender_account()), reserved);
        assert_eq!(token_balance(token, bridge), U256::from(100));
        successful_call(bridge, abi(b"withdraw(uint256)", &[U256::from(100)]));
        assert!(!pallet_assets::Account::<Runtime>::contains_key(
            ATTEST_COIN_ASSET_ID,
            sender_account()
        ));
        assert_eq!(token_balance(token, sender()), U256::from(100));
        assert_eq!(token_balance(token, bridge), U256::zero());
    });
}

#[test]
fn failed_burn_rolls_back_token_transfer_when_parent_catches_revert() {
    ext().execute_with(|| {
        migrate();
        let token = deploy_token();
        let bridge = attest_coin_precompile_h160();
        let parent = H160::repeat_byte(0xBB);
        let parent_account = AddressMapping::into_account_id(parent);
        // Relay calldata to the bridge, store its success bit at slot 0, commit a marker at
        // slot 1, and RETURN its revert data. The parent succeeds even if the child reverts.
        let relay = hex_literal::hex!(
            "
            36 60 00 60 00 37
            60 00 60 00 36 60 00 60 00 61 0f d5 5a f1
            60 00 55 60 01 60 01 55
            3d 60 00 60 00 3e 3d 60 00 f3
        "
        );
        assert_ok!(EVM::create_account(parent, relay.to_vec(), None));
        assert_ok!(Balances::force_set_balance(
            RuntimeOrigin::root(),
            NativeOrEvmAddressLookup::unlookup(parent_account.clone()),
            CTC
        ));
        assert_ok!(Assets::mint(
            RuntimeOrigin::signed(attest_coin_precompile_account()),
            ATTEST_COIN_ASSET_ID,
            NativeOrEvmAddressLookup::unlookup(parent_account.clone()),
            100
        ));
        successful_call(
            token,
            abi(
                b"mint(address,uint256)",
                &[address_word(bridge), U256::from(100)],
            ),
        );
        assert_ok!(Assets::force_asset_status(
            RuntimeOrigin::root(),
            ATTEST_COIN_ASSET_ID,
            NativeOrEvmAddressLookup::unlookup(sender_account()),
            NativeOrEvmAddressLookup::unlookup(attest_coin_precompile_account()),
            NativeOrEvmAddressLookup::unlookup(sender_account()),
            NativeOrEvmAddressLookup::unlookup(sender_account()),
            1,
            false,
            false
        ));
        let info = successful_call(parent, abi(b"withdraw(uint256)", &[U256::from(100)]));
        assert!(
            info.value
                .windows(b"NoPermission".len())
                .any(|w| w == b"NoPermission"),
            "{info:?}"
        );
        assert_eq!(
            pallet_evm::AccountStorages::<Runtime>::get(parent, Hash::zero()),
            Hash::zero()
        );
        assert_eq!(
            pallet_evm::AccountStorages::<Runtime>::get(parent, Hash::from_low_u64_be(1)),
            Hash::from_low_u64_be(1)
        );
        assert!(info.logs.is_empty());
        assert_eq!(Assets::balance(ATTEST_COIN_ASSET_ID, parent_account), 100);
        assert_eq!(token_balance(token, bridge), U256::from(100));
        assert_eq!(token_balance(token, parent), U256::zero());
    });
}
