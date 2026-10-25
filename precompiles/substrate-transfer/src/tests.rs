use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    SubstrateTransferPrecompile, SELECTOR_LOG_TRANSFER,
};

use fp_evm::{ExitError, ExitReason};
use frame_support::storage::unhashed;
use pallet_evm::{CallInfo, GasWeightMapping, Runner};
use precompile_utils::{evm::logs::log3, solidity, testing::*};
use sp_core::{H160, H256, U256};
use std::str::from_utf8;

// No test of invalid selectors since we have a fallback behavior (deposit).
fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

#[test]
fn transfer_substrate_when_sender_has_enough_funds_should_work() {
    let alice: H160 = Alice.into();

    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // lock
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::transfer_substrate {
                        destination: bob_account,
                        amount: 200.into(),
                    },
                )
                .expect_log(log3(
                    Precompile,
                    SELECTOR_LOG_TRANSFER,
                    H256::from(alice),
                    bob_account,
                    solidity::encode_event_data(200_u128),
                ))
                .execute_returns(true);

            // Alice --
            let alice: Account = alice.into();
            let alice_balance = Balances::free_balance(alice);
            assert_eq!(alice_balance, 100);

            // Bob ++
            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 200);
        });
}

#[test]
fn transfer_substrate_when_sender_has_insufficient_funds_should_error() {
    let alice: H160 = Alice.into();

    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (Bob, 10)])
        .build()
        .execute_with(|| {
            // lock
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::transfer_substrate {
                        destination: bob_account,
                        amount: 400.into(),
                    },
                )
                .execute_reverts(|output| {
                    from_utf8(output)
                        .unwrap()
                        .contains("Dispatched call failed with error: ")
                        && from_utf8(output).unwrap().contains("Arithmetic(Underflow)")
                });

            // Alice - no change
            let alice: Account = alice.into();
            let alice_balance = Balances::free_balance(alice);
            assert_eq!(alice_balance, 300);

            // Bob - no change
            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 10);
        });
}

#[test]
fn transfer_substrate_when_amount_gt_max_balance_should_error() {
    let alice: H160 = Alice.into();

    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (Bob, 10)])
        .build()
        .execute_with(|| {
            // lock
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::transfer_substrate {
                        destination: bob_account,
                        // note: Substrate Balance is U128 but the argument here is U256!
                        amount: U256::MAX,
                    },
                )
                .execute_reverts(|output| {
                    // note: the Substrate call never gets dispatched in this case
                    from_utf8(output)
                        .unwrap()
                        .contains("Value is too large for balance type")
                });

            // Alice - no change
            let alice: Account = alice.into();
            let alice_balance = Balances::free_balance(alice);
            assert_eq!(alice_balance, 300);

            // Bob - no change
            let bob: Account = bob_account.0.into();
            let bob_balance = Balances::free_balance(bob);
            assert_eq!(bob_balance, 10);
        });
}

/// Runs `transfer_substrate` through the production EVM runner, so the storage meter is live,
/// with a gas limit that leaves `storage_limit` bytes of storage growth.
fn run_transfer(sender: H160, destination: H256, amount: u128, storage_limit: u64) -> CallInfo {
    let gas_limit = storage_limit * GasLimitStorageGrowthRatio::get();
    let call = PCall::transfer_substrate {
        destination,
        amount: amount.into(),
    };
    <Runtime as pallet_evm::Config>::Runner::call(
        sender,
        Precompile.into(),
        call.into(),
        U256::zero(),
        gas_limit,
        Some(U256::zero()),
        Some(U256::zero()),
        None,
        vec![],
        vec![],
        true,
        false,
        Some(<Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(gas_limit, true)),
        Some(0),
        <Runtime as pallet_evm::Config>::config(),
    )
    .expect("the runner accepts the call")
}

fn new_account_growth() -> u64 {
    SubstrateTransferPrecompile::<Runtime>::new_account_storage_growth()
}

/// Bytes the `System::Account` entry of `who` occupies: storage key plus value.
fn account_footprint(who: &Account) -> u64 {
    let key = frame_system::Account::<Runtime>::hashed_key_for(who);
    let value = unhashed::get_raw(&key).expect("account is stored");
    (key.len() + value.len()) as u64
}

#[test]
fn new_account_allowance_covers_the_stored_account_entry() {
    let alice: H160 = Alice.into();
    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            let info = run_transfer(alice, bob_account, 200, new_account_growth());
            assert!(info.exit_reason.is_succeed());

            assert!(new_account_growth() >= account_footprint(&Bob));
        });
}

#[test]
fn transfer_to_fresh_destination_is_metered_as_storage_growth() {
    let alice: H160 = Alice.into();
    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // One byte short of the allowance: the meter rejects the transfer and nothing persists.
            let info = run_transfer(alice, bob_account, 200, new_account_growth() - 1);
            assert_eq!(info.exit_reason, ExitReason::Error(ExitError::OutOfGas));
            assert!(!System::account_exists(&Bob));
            assert_eq!(Balances::free_balance(Alice), 300);

            // Exactly the allowance is enough.
            let info = run_transfer(alice, bob_account, 200, new_account_growth());
            assert!(info.exit_reason.is_succeed());
            assert_eq!(Balances::free_balance(Alice), 100);
            assert_eq!(Balances::free_balance(Bob), 200);
        });
}

#[test]
fn transfer_to_existing_destination_declares_no_storage_growth() {
    let alice: H160 = Alice.into();
    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300), (Bob, 10)])
        .build()
        .execute_with(|| {
            // The limit that rejects a fresh destination is ample for an existing one.
            let info = run_transfer(alice, bob_account, 200, new_account_growth() - 1);
            assert!(info.exit_reason.is_succeed());
            assert_eq!(Balances::free_balance(Alice), 100);
            assert_eq!(Balances::free_balance(Bob), 210);
        });
}

#[test]
fn failed_transfer_to_fresh_destination_leaves_no_account_behind() {
    let alice: H160 = Alice.into();
    let bob_account: H256 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(alice.into(), 300)])
        .build()
        .execute_with(|| {
            // Storage budget is ample, so the failure is the dispatch's own (insufficient funds).
            let info = run_transfer(alice, bob_account, 400, new_account_growth());
            assert!(matches!(info.exit_reason, ExitReason::Revert(_)));
            assert!(!System::account_exists(&Bob));
            assert_eq!(Balances::free_balance(Alice), 300);
        });
}

#[test]
fn storage_growth_allowance_does_not_change_proof_size_accounting() {
    let alice: H160 = Alice.into();
    let bob_account: H256 = Bob.into();

    let proof_size_usage = |bob_exists: bool| {
        let balances = if bob_exists {
            vec![(alice.into(), 300), (Bob, 10)]
        } else {
            vec![(alice.into(), 300)]
        };
        ExtBuilder::default()
            .with_balances(balances)
            .build()
            .execute_with(|| {
                let info = run_transfer(alice, bob_account, 200, new_account_growth());
                assert!(info.exit_reason.is_succeed());
                info.weight_info
                    .and_then(|w| w.proof_size_usage)
                    .expect("proof size is tracked")
            })
    };

    assert_eq!(proof_size_usage(false), proof_size_usage(true));
}
