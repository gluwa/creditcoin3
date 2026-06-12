use crate::mock::RuntimeEvent;
use crate::{mock::*, pallet::*, Error};
use frame_support::{assert_noop, assert_ok};
use sp_core::H160;
use sp_runtime::traits::Zero;

const ERC20: H160 = H160([
    0xE0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
]);

// ── set_attest_coin_token ─────────────────────────────────────────────────────

#[test]
fn set_attest_coin_token_works_for_root() {
    new_test_ext().execute_with(|| {
        assert!(AttestCoinErc20::<Runtime>::get().is_none());
        assert_ok!(crate::Pallet::<Runtime>::set_attest_coin_token(
            frame_system::RawOrigin::Root.into(),
            ERC20
        ));
        assert_eq!(AttestCoinErc20::<Runtime>::get(), Some(ERC20));
    });
}

#[test]
fn set_attest_coin_token_rejects_non_root() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            crate::Pallet::<Runtime>::set_attest_coin_token(
                frame_system::RawOrigin::Signed(alice()).into(),
                ERC20
            ),
            frame_support::error::BadOrigin
        );
    });
}

// ── accrued_of / restore_accrued ─────────────────────────────────────────────

#[test]
fn accrued_of_returns_zero_for_unknown_account() {
    new_test_ext().execute_with(|| {
        assert!(crate::Pallet::<Runtime>::accrued_of(&alice()).is_zero());
    });
}

#[test]
fn restore_accrued_adds_to_balance() {
    new_test_ext().execute_with(|| {
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 500);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 500);

        crate::Pallet::<Runtime>::restore_accrued(&alice(), 300);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 800);
    });
}

// ── take_accrued_for_claim ────────────────────────────────────────────────────

#[test]
fn take_accrued_for_claim_works_without_ledger_entry() {
    new_test_ext().execute_with(|| {
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 500);
        assert_ok!(crate::Pallet::<Runtime>::take_accrued_for_claim(
            &alice(),
            200
        ));
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 300);
    });
}

#[test]
fn take_accrued_for_claim_fails_if_insufficient() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 50);

        assert!(
            matches!(
                crate::Pallet::<Runtime>::take_accrued_for_claim(&alice(), 100),
                Err(Error::InsufficientAccrued)
            ),
            "expected InsufficientAccrued"
        );
        // Accrued should be unchanged after failed attempt
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 50);
    });
}

#[test]
fn take_accrued_for_claim_deducts_correctly() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 500);

        assert_ok!(crate::Pallet::<Runtime>::take_accrued_for_claim(
            &alice(),
            200
        ));
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 300);
    });
}

// ── commit_claim / undo_claim_commit ──────────────────────────────────────────

#[test]
fn commit_claim_fails_on_wrong_nonce() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 1_000);

        // nonce is 0, passing 1 should fail
        assert!(
            matches!(
                crate::Pallet::<Runtime>::commit_claim(&alice(), 1, 100),
                Err(Error::BadClaimNonce)
            ),
            "expected BadClaimNonce"
        );
        // nonce should not have changed
        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), 0);
    });
}

#[test]
fn commit_claim_fails_on_insufficient_accrued() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 50);

        assert!(
            matches!(
                crate::Pallet::<Runtime>::commit_claim(&alice(), 0, 100),
                Err(Error::InsufficientAccrued)
            ),
            "expected InsufficientAccrued"
        );
        // Nonce and accrued unchanged
        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), 0);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 50);
    });
}

#[test]
fn commit_claim_deducts_and_bumps_nonce() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 1_000);

        assert_ok!(crate::Pallet::<Runtime>::commit_claim(&alice(), 0, 400));
        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), 1);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 600);

        // Second claim with incremented nonce
        assert_ok!(crate::Pallet::<Runtime>::commit_claim(&alice(), 1, 200));
        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), 2);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 400);
    });
}

#[test]
fn commit_claim_rejects_max_nonce_without_debiting_accrued() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 1_000);
        ClaimNonce::<Runtime>::insert(alice(), u64::MAX);

        assert!(
            matches!(
                crate::Pallet::<Runtime>::commit_claim(&alice(), u64::MAX, 100),
                Err(Error::BadClaimNonce)
            ),
            "expected BadClaimNonce when nonce cannot be incremented"
        );
        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), u64::MAX);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 1_000);
    });
}

#[test]
fn undo_claim_commit_restores_nonce_and_accrued() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 1_000);

        // Simulate a successful commit followed by an EVM transfer failure
        assert_ok!(crate::Pallet::<Runtime>::commit_claim(&alice(), 0, 400));
        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), 1);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 600);

        crate::Pallet::<Runtime>::undo_claim_commit(&alice(), 0, 400);

        assert_eq!(crate::Pallet::<Runtime>::claim_nonce_of(&alice()), 0);
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 1_000);
    });
}

#[test]
fn nonce_replay_is_rejected_after_commit() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        crate::Pallet::<Runtime>::restore_accrued(&alice(), 1_000);

        assert_ok!(crate::Pallet::<Runtime>::commit_claim(&alice(), 0, 100));

        // Replaying nonce 0 must fail
        assert!(
            matches!(
                crate::Pallet::<Runtime>::commit_claim(&alice(), 0, 100),
                Err(Error::BadClaimNonce)
            ),
            "expected BadClaimNonce on replay"
        );
    });
}

// ── reward_commit_signers ─────────────────────────────────────────────────────

#[test]
fn reward_commit_signers_is_noop_for_empty_list() {
    new_test_ext().execute_with(|| {
        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[]);
        // No accrual happened
        assert!(crate::Pallet::<Runtime>::accrued_of(&alice()).is_zero());
    });
}

#[test]
fn reward_commit_signers_credits_stash_for_registered_attestor() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        register_attestor(CHAIN_KEY, bob(), alice()); // bob is operator, alice is stash

        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);

        // alice (stash) should have received 100 points (RewardPerEligibleSigner)
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 100);
        // bob (operator, not stash in this entry) gets nothing directly
        assert!(crate::Pallet::<Runtime>::accrued_of(&bob()).is_zero());
    });
}

#[test]
fn reward_commit_signers_skips_unregistered_attestors() {
    new_test_ext().execute_with(|| {
        // bob is not registered as an attestor for CHAIN_KEY
        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);
        assert!(crate::Pallet::<Runtime>::accrued_of(&alice()).is_zero());
        assert!(crate::Pallet::<Runtime>::accrued_of(&bob()).is_zero());
    });
}

#[test]
fn reward_commit_signers_emits_skipped_event_for_unregistered_attestors() {
    new_test_ext().execute_with(|| {
        // bob is not a registered attestor. The reward path must surface this
        // desync via `RewardSkippedNoStash` so it's observable on operator dashboards
        // even when no real reward credit happens.
        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);

        System::assert_has_event(
            crate::pallet::Event::<Runtime>::RewardSkippedNoStash {
                chain_key: CHAIN_KEY,
                skipped: 1,
            }
            .into(),
        );
    });
}

#[test]
fn reward_commit_signers_emits_skipped_event_independent_of_token_config() {
    new_test_ext().execute_with(|| {
        // The CommitSignersRewarded event is gated on AttestCoinErc20 being set
        // (it's a reward-program-active signal). The desync signal must NOT be gated —
        // operators need to see desyncs regardless of claim-program activation.
        assert!(AttestCoinErc20::<Runtime>::get().is_none());
        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);

        System::assert_has_event(
            crate::pallet::Event::<Runtime>::RewardSkippedNoStash {
                chain_key: CHAIN_KEY,
                skipped: 1,
            }
            .into(),
        );
    });
}

#[test]
fn reward_commit_signers_emits_event_when_token_configured() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        register_attestor(CHAIN_KEY, bob(), alice());
        AttestCoinErc20::<Runtime>::put(ERC20);

        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);

        System::assert_has_event(
            crate::pallet::Event::<Runtime>::CommitSignersRewarded {
                chain_key: CHAIN_KEY,
                signers: 1,
                per_signer: 100,
            }
            .into(),
        );
    });
}

#[test]
fn reward_commit_signers_does_not_emit_event_without_token() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        register_attestor(CHAIN_KEY, bob(), alice());
        // ERC20 not set → no event, but accrual still happens

        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);

        // Accrued but no event
        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 100);
        let events = System::events();
        assert!(
            events.iter().all(|e| !matches!(
                e.event,
                RuntimeEvent::AttestCoinRewards(crate::pallet::Event::CommitSignersRewarded { .. })
            )),
            "should not emit CommitSignersRewarded without ERC-20 configured"
        );
    });
}

#[test]
fn reward_commit_signers_accumulates_across_multiple_calls() {
    new_test_ext().execute_with(|| {
        register_stash(alice());
        register_attestor(CHAIN_KEY, bob(), alice());

        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);
        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);
        crate::Pallet::<Runtime>::reward_commit_signers(CHAIN_KEY, &[bob()]);

        assert_eq!(crate::Pallet::<Runtime>::accrued_of(&alice()), 300);
    });
}

// ── claim_signing_message ─────────────────────────────────────────────────────

#[test]
fn claim_signing_message_has_correct_length() {
    new_test_ext().execute_with(|| {
        let msg = crate::Pallet::<Runtime>::claim_signing_message(
            &alice(),
            0,
            CHAIN_KEY,
            1_000,
            [0u8; 20],
        );
        // b"AttestCoin:claim:v2:" (20) + genesis(32) + stash(32) + nonce(8) + chain_key(8)
        // + amount(16) + recipient(20)
        assert_eq!(msg.len(), 20 + 32 + 32 + 8 + 8 + 16 + 20);
    });
}

#[test]
fn claim_signing_message_binds_genesis_hash() {
    // The v2 message embeds the network genesis hash for cross-network replay protection
    // (audit Low #1). Assert the genesis bytes appear immediately after the 20-byte prefix.
    new_test_ext().execute_with(|| {
        let msg = crate::Pallet::<Runtime>::claim_signing_message(
            &alice(),
            0,
            CHAIN_KEY,
            1_000,
            [0u8; 20],
        );
        let genesis = frame_system::Pallet::<Runtime>::block_hash(0u32);
        assert_eq!(&msg[20..52], genesis.as_ref());
    });
}

#[test]
fn claim_signing_message_changes_with_nonce() {
    new_test_ext().execute_with(|| {
        let msg0 = crate::Pallet::<Runtime>::claim_signing_message(&alice(), 0, 1, 100, [0u8; 20]);
        let msg1 = crate::Pallet::<Runtime>::claim_signing_message(&alice(), 1, 1, 100, [0u8; 20]);
        assert_ne!(msg0, msg1);
    });
}
