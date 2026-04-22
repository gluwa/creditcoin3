use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    AttestorInfo, LedgerInfo, SELECTOR_LOG_ATTESTOR_CHILLED, SELECTOR_LOG_ATTESTOR_REGISTERED,
    SELECTOR_LOG_ATTESTOR_UNREGISTERED, SELECTOR_LOG_UNBONDED_WITHDRAWN,
};

use precompile_utils::{evm::logs::log2, evm::logs::log4, testing::*};
use sp_core::{H160, H256};
use std::str::from_utf8;

fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

/// 100 units with 18-decimals precision, matching `DefaultMinBondRequirement`.
const MIN_BOND: u128 = 100_000_000_000_000_000_000;

fn attestor_id() -> H256 {
    let a: H160 = crate::mock::Account::AttestorA.into();
    a.into()
}

fn attestor_id_b() -> H256 {
    let a: H160 = crate::mock::Account::AttestorB.into();
    a.into()
}

#[test]
fn register_attestor_with_sufficient_bond_should_succeed_and_emit_event() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .expect_log(log4(
                    Precompile,
                    SELECTOR_LOG_ATTESTOR_REGISTERED,
                    H256::from_low_u64_be(TEST_CHAIN_KEY),
                    attestor_id(),
                    H256::from(alice),
                    Vec::<u8>::new(),
                ))
                .execute_returns(true);
        });
}

#[test]
fn register_attestor_without_balance_should_revert() {
    let alice: H160 = Alice.into();

    // Intentionally *not* endowing Alice so that the pallet's bond check fails.
    ExtBuilder::default()
        .with_balances(vec![])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("InsufficientBalance")
                });
        });
}

#[test]
fn register_attestor_for_unsupported_chain_should_revert() {
    let alice: H160 = Alice.into();
    let unsupported_chain: u64 = 42;

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: unsupported_chain,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("ChainNotSupported")
                });
        });
}

#[test]
fn unregister_attestor_not_registered_should_revert() {
    let bob: H160 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(Bob, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    bob,
                    Precompile,
                    PCall::unregister_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id_b(),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("AddressNotAttestor")
                });
        });
}

#[test]
fn register_then_unregister_attestor_should_succeed_and_emit_events() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .expect_log(log4(
                    Precompile,
                    SELECTOR_LOG_ATTESTOR_REGISTERED,
                    H256::from_low_u64_be(TEST_CHAIN_KEY),
                    attestor_id(),
                    H256::from(alice),
                    Vec::<u8>::new(),
                ))
                .execute_returns(true);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::unregister_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .expect_log(log4(
                    Precompile,
                    SELECTOR_LOG_ATTESTOR_UNREGISTERED,
                    H256::from_low_u64_be(TEST_CHAIN_KEY),
                    attestor_id(),
                    H256::from(alice),
                    Vec::<u8>::new(),
                ))
                .execute_returns(true);
        });
}

#[test]
fn chill_unknown_attestor_should_revert() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::chill {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("AddressNotAttestor")
                });
        });
}

#[test]
fn chill_attestor_from_non_stash_should_revert() {
    // Alice registers her attestor; Bob (who isn't the stash) tries to chill it.
    let alice: H160 = Alice.into();
    let bob: H160 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND), (Bob, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            precompiles()
                .prepare_test(
                    bob,
                    Precompile,
                    PCall::chill {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("NotYourAttestor")
                });
        });
}

#[test]
fn register_then_chill_attestor_should_succeed_and_emit_event() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::chill {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .expect_log(log4(
                    Precompile,
                    SELECTOR_LOG_ATTESTOR_CHILLED,
                    H256::from_low_u64_be(TEST_CHAIN_KEY),
                    attestor_id(),
                    H256::from(alice),
                    Vec::<u8>::new(),
                ))
                .execute_returns(true);
        });
}

#[test]
fn withdraw_unbonded_with_nothing_to_withdraw_should_revert() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(alice, Precompile, PCall::withdraw_unbonded {})
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ") && s.contains("NotStash")
                });
        });
}

#[test]
fn get_attestor_not_registered_returns_default() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(AttestorInfo::default());
        });
}

#[test]
fn get_attestor_after_register_returns_info() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            // AttestorA is its own stash in mock (Alice registers AttestorA)
            // stash in ledger = Alice
            let alice_h256: H256 = Alice.into();
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(AttestorInfo {
                    exists: true,
                    status: 1, // Idle
                    stash: alice_h256,
                    has_bls_key: false,
                });
        });
}

#[test]
fn is_active_attestor_returns_false_after_register() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::is_active_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(false);
        });
}

#[test]
fn get_attestors_count_after_register() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestors_count {
                        chain_key: TEST_CHAIN_KEY,
                    },
                )
                .execute_returns(0u32);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_attestors_count {
                        chain_key: TEST_CHAIN_KEY,
                    },
                )
                .execute_returns(1u32);
        });
}

#[test]
fn get_ledger_after_register_returns_staked_amount() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            // No ledger before register
            let alice_h256: H256 = Alice.into();
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_ledger { stash: alice_h256 })
                .execute_returns(LedgerInfo::default());

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            precompiles()
                .prepare_test(alice, Precompile, PCall::get_ledger { stash: alice_h256 })
                .execute_returns(LedgerInfo {
                    exists: true,
                    total_staked: MIN_BOND,
                    active: MIN_BOND,
                    unlocking_chunks: 0,
                });
        });
}

#[test]
fn get_min_bond_requirement_returns_default() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_min_bond_requirement {
                        chain_key: TEST_CHAIN_KEY,
                    },
                )
                .execute_returns(MIN_BOND);
        });
}

#[test]
fn withdraw_unbonded_after_unregister_emits_event() {
    let alice: H160 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::register_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::unregister_attestor {
                        chain_key: TEST_CHAIN_KEY,
                        attestor_id: attestor_id(),
                    },
                )
                .execute_returns(true);

            // Advance past the bonding duration so `withdraw_unbonded` can
            // release the locked funds.  `BondingDuration` is 3 eras in the
            // test mock; we write `CurrentEra` directly rather than running the
            // full session/babe machinery.
            let bonding_duration = <Runtime as pallet_attestation::Config>::BondingDuration::get();
            pallet_staking::CurrentEra::<Runtime>::put(bonding_duration + 1);

            precompiles()
                .prepare_test(alice, Precompile, PCall::withdraw_unbonded {})
                .expect_log(log2(
                    Precompile,
                    SELECTOR_LOG_UNBONDED_WITHDRAWN,
                    H256::from(alice),
                    Vec::<u8>::new(),
                ))
                .execute_returns(true);
        });
}
