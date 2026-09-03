use crate::{
    mock::{
        Account::{Alice, Bob, Precompile},
        *,
    },
    AttestorInfo, LedgerInfo, SELECTOR_LOG_ATTESTOR_CHILLED, SELECTOR_LOG_ATTESTOR_REGISTERED,
    SELECTOR_LOG_ATTESTOR_UNREGISTERED, SELECTOR_LOG_BOND_EXTRA_ADDED,
    SELECTOR_LOG_SURPLUS_UNBONDED, SELECTOR_LOG_UNBONDED_WITHDRAWN,
};

use precompile_utils::{evm::logs::log2, evm::logs::log4, solidity, testing::*};
use sp_core::{H160, H256, U256};
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
fn register_then_chill_idle_attestor_should_revert() {
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
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("AttestorAlreadyIdle")
                });
        });
}

#[test]
fn chill_active_attestor_should_schedule_leaving_and_emit_log() {
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

            pallet_attestation::Attestors::<Runtime>::mutate(
                TEST_CHAIN_KEY,
                &crate::mock::Account::AttestorA,
                |maybe_attestor| {
                    maybe_attestor.as_mut().expect("registered above").status =
                        attestor_primitives::AttestorStatus::Active;
                },
            );
            pallet_attestation::ActiveAttestors::<Runtime>::insert(
                TEST_CHAIN_KEY,
                vec![crate::mock::Account::AttestorA],
            );

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
                    status: 3, // Leaving
                    stash: H256::from(alice),
                    has_bls_key: false,
                });
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
                    stash: alice_h256,
                    total_staked: MIN_BOND,
                    active: MIN_BOND,
                    unlocking_chunks: 0,
                    withdrawable: 0,
                });
        });
}

#[test]
fn get_ledger_by_address_returns_same_as_get_ledger() {
    use precompile_utils::solidity::codec::Address;

    let alice: H160 = Alice.into();
    let alice_h256: H256 = Alice.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            // No ledger yet — both entries should return the default `LedgerInfo`.
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_ledger_by_address {
                        addr: Address(alice),
                    },
                )
                .execute_returns(LedgerInfo::default());

            // Register so a ledger exists.
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

            // `getLedgerByAddress(address)` must return exactly the same `LedgerInfo` as
            // `getLedger(bytes32)` does for the corresponding hashed AccountId. This is the
            // whole point of the new entry: EVM consumers shouldn't have to know about the
            // AddressMapping translation to read their own ledger.
            let expected = LedgerInfo {
                exists: true,
                stash: alice_h256,
                total_staked: MIN_BOND,
                active: MIN_BOND,
                unlocking_chunks: 0,
                withdrawable: 0,
            };
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_ledger { stash: alice_h256 })
                .execute_returns(expected.clone());
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::get_ledger_by_address {
                        addr: Address(alice),
                    },
                )
                .execute_returns(expected);
        });
}

#[test]
fn get_caller_ledger_uses_msg_sender() {
    let alice: H160 = Alice.into();
    let bob: H160 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(Alice, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            // Bob never registered, so `getCallerLedger()` from Bob must return the default.
            // Alice did register, so `getCallerLedger()` from Alice must return her ledger.
            // This is the load-bearing assertion: the entry resolves the *caller's* address
            // through AddressMapping rather than reading some other account's ledger.
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

            let alice_h256: H256 = Alice.into();
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_caller_ledger {})
                .execute_returns(LedgerInfo {
                    exists: true,
                    stash: alice_h256,
                    total_staked: MIN_BOND,
                    active: MIN_BOND,
                    unlocking_chunks: 0,
                    withdrawable: 0,
                });

            precompiles()
                .prepare_test(bob, Precompile, PCall::get_caller_ledger {})
                .execute_returns(LedgerInfo::default());
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

// ── bondExtra / unbondSurplus ──────────────────────────────────────────────────
//
// These two exist on the precompile because an EVM-space stash has no signing key, so the
// Substrate extrinsics are unreachable for it. `unbond_surplus` in particular is the only release
// path for bond above the aggregate requirement — see the regression test at the bottom.

#[test]
fn bond_extra_tops_up_the_ledger_and_emits_event() {
    let alice: H160 = Alice.into();
    let alice_h256: H256 = Alice.into();

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

            let top_up = U256::from(MIN_BOND);
            precompiles()
                .prepare_test(alice, Precompile, PCall::bond_extra { amount: top_up })
                .expect_log(log2(
                    Precompile,
                    SELECTOR_LOG_BOND_EXTRA_ADDED,
                    H256::from(alice),
                    solidity::encode_event_data(top_up),
                ))
                .execute_returns(true);

            precompiles()
                .prepare_test(alice, Precompile, PCall::get_caller_ledger {})
                .execute_returns(LedgerInfo {
                    exists: true,
                    stash: alice_h256,
                    total_staked: 2 * MIN_BOND,
                    active: 2 * MIN_BOND,
                    unlocking_chunks: 0,
                    withdrawable: 0,
                });
        });
}

#[test]
fn bond_extra_without_a_ledger_reverts() {
    let bob: H160 = Bob.into();

    ExtBuilder::default()
        .with_balances(vec![(Bob, 10 * MIN_BOND)])
        .build()
        .execute_with(|| {
            // Bob is funded but has never registered an attestor, so there is no ledger to top up.
            precompiles()
                .prepare_test(
                    bob,
                    Precompile,
                    PCall::bond_extra {
                        amount: U256::from(MIN_BOND),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ") && s.contains("NotStash")
                });
        });
}

#[test]
fn bond_extra_above_the_balance_type_width_reverts_without_truncating() {
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

            // Must revert, not wrap: a truncated `U256::MAX` would bond an arbitrary small amount.
            precompiles()
                .prepare_test(alice, Precompile, PCall::bond_extra { amount: U256::MAX })
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Value is too large for uint128")
                });
        });
}

#[test]
fn unbond_surplus_releases_the_top_up_and_emits_event() {
    let alice: H160 = Alice.into();
    let alice_h256: H256 = Alice.into();

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

            let amount = U256::from(MIN_BOND);
            precompiles()
                .prepare_test(alice, Precompile, PCall::bond_extra { amount })
                .execute_returns(true);

            precompiles()
                .prepare_test(alice, Precompile, PCall::unbond_surplus { amount })
                .expect_log(log2(
                    Precompile,
                    SELECTOR_LOG_SURPLUS_UNBONDED,
                    H256::from(alice),
                    solidity::encode_event_data(amount),
                ))
                .execute_returns(true);

            // `active` drops back to the requirement; the released amount is now unlocking and
            // `total_staked` stays put until `withdrawUnbonded` moves it out of the pool.
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_caller_ledger {})
                .execute_returns(LedgerInfo {
                    exists: true,
                    stash: alice_h256,
                    total_staked: 2 * MIN_BOND,
                    active: MIN_BOND,
                    unlocking_chunks: 1,
                    withdrawable: 0,
                });
        });
}

#[test]
fn unbond_surplus_cannot_breach_the_aggregate_requirement() {
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

            // `active` is exactly the requirement for the one registered attestor, so there is no
            // surplus to release — the solvency guard must reject this rather than undercollateralize.
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::unbond_surplus {
                        amount: U256::from(MIN_BOND),
                    },
                )
                .execute_reverts(|output| {
                    let s = from_utf8(output).unwrap();
                    s.contains("Dispatched call failed with error: ")
                        && s.contains("InsufficientRemainingBond")
                });
        });
}

/// Regression for the reason `unbondSurplus` has to be on this precompile at all.
///
/// An EVM-space stash cannot sign a Substrate extrinsic, so before this entry existed a governance
/// *decrease* of `MinBondRequirement` stranded the difference in the bond pool permanently:
/// `unregister_attestor` releases at most the *current* requirement, and `withdraw_unbonded` only
/// reaps once `active` is below the existential deposit.
#[test]
fn unbond_surplus_releases_bond_stranded_by_a_min_bond_decrease() {
    let alice: H160 = Alice.into();
    let alice_h256: H256 = Alice.into();

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

            // Governance halves the requirement after the stash already bonded the full amount.
            let lowered = MIN_BOND / 2;
            pallet_attestation::MinBondRequirement::<Runtime>::set(TEST_CHAIN_KEY, lowered);

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

            // Unregister released only the *lowered* requirement, leaving `MIN_BOND - lowered`
            // stuck in `active` with no attestor left to justify it.
            let stranded = MIN_BOND - lowered;
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_caller_ledger {})
                .execute_returns(LedgerInfo {
                    exists: true,
                    stash: alice_h256,
                    total_staked: MIN_BOND,
                    active: stranded,
                    unlocking_chunks: 1,
                    withdrawable: 0,
                });

            // With no attestors registered the aggregate requirement is zero, so the remainder is
            // fully releasable — the point of the entry.
            precompiles()
                .prepare_test(
                    alice,
                    Precompile,
                    PCall::unbond_surplus {
                        amount: U256::from(stranded),
                    },
                )
                .execute_returns(true);

            let bonding_duration = <Runtime as pallet_attestation::Config>::BondingDuration::get();
            pallet_staking::CurrentEra::<Runtime>::put(bonding_duration + 1);

            precompiles()
                .prepare_test(alice, Precompile, PCall::withdraw_unbonded {})
                .execute_returns(true);

            // Ledger reaped: nothing of the original bond is left behind in the pool.
            precompiles()
                .prepare_test(alice, Precompile, PCall::get_caller_ledger {})
                .execute_returns(LedgerInfo {
                    exists: false,
                    stash: H256::zero(),
                    total_staked: 0,
                    active: 0,
                    unlocking_chunks: 0,
                    withdrawable: 0,
                });
        });
}
