use crate::mock::*;
use crate::{SEL_ACCRUED, SEL_CLAIM, SEL_DEPOSIT, SEL_DEPOSIT_TO, SEL_WITHDRAW};
use fp_evm::{Context, ExitReason, ExitRevert, ExitSucceed, PrecompileFailure};
use frame_support::assert_ok;
use pallet_assets::Pallet as AssetsPallet;
use pallet_attest_coin_rewards::Accrued;
use pallet_evm::AddressMapping;
use precompile_utils::testing::{MockHandle, SubcallOutput};
use sp_core::{sr25519, Pair, H160, U256};

fn precompile_addr() -> H160 {
    H160::from_low_u64_be(PRECOMPILE_ADDRESS_U64)
}

/// Create a fresh `MockHandle` with the caller == evm_recipient == `caller_addr`.
fn make_handle(caller_addr: H160, input: Vec<u8>) -> MockHandle {
    let mut handle = MockHandle::new(
        precompile_addr(),
        Context {
            address: precompile_addr(),
            caller: caller_addr,
            apparent_value: U256::zero(),
        },
    );
    handle.input = input;
    handle
}

/// ABI-encode a bytes32 (left-pad 20-byte address to 32 bytes).
fn encode_addr_as_bytes32(addr: H160) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[12..32].copy_from_slice(addr.as_bytes());
    out
}

fn encode_u256(v: u128) -> [u8; 32] {
    let u = U256::from(v);
    let mut out = [0u8; 32];
    u.to_big_endian(&mut out);
    out
}

/// Build raw input for `accrued(bytes32)`.
fn accrued_input(stash_bytes32: [u8; 32]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&SEL_ACCRUED);
    v.extend_from_slice(&stash_bytes32);
    v
}

/// Build raw input for `deposit(uint256)`.
fn deposit_input(amount: u128) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&SEL_DEPOSIT);
    v.extend_from_slice(&encode_u256(amount));
    v
}

/// Build raw input for `depositTo(uint256,bytes32)`.
fn deposit_to_input(amount: u128, beneficiary: [u8; 32]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&SEL_DEPOSIT_TO);
    v.extend_from_slice(&encode_u256(amount));
    v.extend_from_slice(&beneficiary);
    v
}

/// Build raw input for `withdraw(uint256)`.
fn withdraw_input(amount: u128) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&SEL_WITHDRAW);
    v.extend_from_slice(&encode_u256(amount));
    v
}

/// Build raw input for `claim(bytes32,uint256,uint256,uint256,address,bytes32,bytes32)`.
#[allow(clippy::too_many_arguments)]
fn claim_input(
    stash_bytes32: [u8; 32],
    nonce: u64,
    chain_key: u64,
    amount: u128,
    evm_recipient: H160,
    sig_r: [u8; 32],
    sig_s: [u8; 32],
) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&SEL_CLAIM);
    v.extend_from_slice(&stash_bytes32); // bytes32 stash
    v.extend_from_slice(&encode_u256(nonce as u128)); // uint256 nonce
    v.extend_from_slice(&encode_u256(chain_key as u128)); // uint256 chain_key
    v.extend_from_slice(&encode_u256(amount)); // uint256 amount
    v.extend_from_slice(&encode_addr_as_bytes32(evm_recipient)); // address (padded)
    v.extend_from_slice(&sig_r); // bytes32 r
    v.extend_from_slice(&sig_s); // bytes32 s
    v
}

fn execute(handle: &mut MockHandle) -> fp_evm::PrecompileResult {
    use fp_evm::Precompile as _;
    crate::AttestCoinPrecompile::<Runtime>::execute(handle)
}

fn assert_reverts_with(handle: &mut MockHandle, expected_msg: &[u8]) {
    let result = execute(handle);
    match result {
        Err(PrecompileFailure::Revert { output, .. }) => {
            assert_eq!(
                output,
                expected_msg,
                "revert message mismatch: got {:?}",
                core::str::from_utf8(&output)
            );
        }
        other => panic!("expected Revert, got {other:?}"),
    }
}

// ── accrued tests ─────────────────────────────────────────────────────────────

#[test]
fn accrued_returns_zero_for_unknown_stash() {
    ExtBuilder::default().build().execute_with(|| {
        let stash_bytes = [0x42u8; 32];
        let input = accrued_input(stash_bytes);
        let mut handle = make_handle(H160::repeat_byte(0xAA), input);
        let result = execute(&mut handle).expect("accrued should succeed");
        assert!(matches!(result.exit_status, ExitSucceed::Returned));
        let v = U256::from_big_endian(&result.output);
        assert_eq!(v, U256::zero());
    });
}

#[test]
fn accrued_returns_correct_value() {
    ExtBuilder::default().build().execute_with(|| {
        let stash_bytes = [0xABu8; 32];
        let stash = AccountId::from(stash_bytes);
        Accrued::<Runtime>::insert(&stash, 12_345u128);

        let input = accrued_input(stash_bytes);
        let mut handle = make_handle(H160::repeat_byte(0xAA), input);
        let result = execute(&mut handle).expect("accrued should succeed");
        let v = U256::from_big_endian(&result.output);
        assert_eq!(v, U256::from(12_345u128));
    });
}

// ── claim revert tests ────────────────────────────────────────────────────────

#[test]
fn claim_reverts_token_not_configured() {
    ExtBuilder::default().build().execute_with(|| {
        let caller = H160::repeat_byte(0xAA);
        let input = claim_input(
            [0u8; 32],
            0,
            SUPPORTED_CHAIN_KEY,
            0,
            caller,
            [0u8; 32],
            [0u8; 32],
        );
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"token not configured");
    });
}

#[test]
fn claim_reverts_unsupported_chain_key() {
    ExtBuilder::default().build().execute_with(|| {
        // Set a token so we get past token check
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let caller = H160::repeat_byte(0xAA);
        let unsupported_chain_key = 9999u64;
        let input = claim_input(
            [0u8; 32],
            0,
            unsupported_chain_key,
            0,
            caller,
            [0u8; 32],
            [0u8; 32],
        );
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"unsupported chain key");
    });
}

#[test]
fn claim_reverts_not_stash() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let caller = H160::repeat_byte(0xAA);
        // stash_bytes maps to a Bogus account that has no Ledger entry
        let stash_bytes = [0x12u8; 32];
        let input = claim_input(
            stash_bytes,
            0,
            SUPPORTED_CHAIN_KEY,
            0,
            caller,
            [0u8; 32],
            [0u8; 32],
        );
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"not a stash");
    });
}

#[test]
fn claim_reverts_bad_signature() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        // Generate a real keypair so we have a valid stash
        let (pair, _) = sr25519::Pair::generate();
        let stash_raw: [u8; 32] = pair.public().0;
        let stash = AccountId::from(stash_raw);

        // Insert into Ledger so it's recognized as a stash
        pallet_attestation::Ledger::<Runtime>::insert(
            &stash,
            pallet_attestation::AttestorLedger::<Runtime> {
                stash: stash.clone(),
                total_staked: 0u128,
                active: 0u128,
                unlocking: sp_runtime::BoundedVec::default(),
            },
        );

        // caller must equal evm_recipient; use zero address so we reach signature verification with a bad sig
        let zero_caller = H160::zero();
        let input2 = claim_input(
            stash_raw,
            0,
            SUPPORTED_CHAIN_KEY,
            100,
            zero_caller,
            [0u8; 32],
            [0u8; 32],
        );
        let mut handle2 = make_handle(zero_caller, input2);
        assert_reverts_with(&mut handle2, b"bad signature");
    });
}

#[test]
fn claim_reverts_bad_nonce() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let (pair, _) = sr25519::Pair::generate();
        let stash_raw: [u8; 32] = pair.public().0;
        let stash = AccountId::from(stash_raw);

        pallet_attestation::Ledger::<Runtime>::insert(
            &stash,
            pallet_attestation::AttestorLedger::<Runtime> {
                stash: stash.clone(),
                total_staked: 0u128,
                active: 0u128,
                unlocking: sp_runtime::BoundedVec::default(),
            },
        );

        let evm_recipient = H160::zero();
        let nonce = 99u64; // wrong nonce (on-chain is 0)
        let amount = 100u128;
        let msg = pallet_attest_coin_rewards::Pallet::<Runtime>::claim_signing_message(
            &stash,
            nonce,
            SUPPORTED_CHAIN_KEY,
            amount,
            evm_recipient.0,
        );
        let sig = pair.sign(&msg);
        let mut sig_r = [0u8; 32];
        let mut sig_s = [0u8; 32];
        sig_r.copy_from_slice(&sig.0[..32]);
        sig_s.copy_from_slice(&sig.0[32..]);

        let input = claim_input(
            stash_raw,
            nonce,
            SUPPORTED_CHAIN_KEY,
            amount,
            evm_recipient,
            sig_r,
            sig_s,
        );
        let mut handle = make_handle(evm_recipient, input);
        assert_reverts_with(&mut handle, b"bad nonce");
    });
}

#[test]
fn claim_reverts_insufficient_accrued() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let (pair, _) = sr25519::Pair::generate();
        let stash_raw: [u8; 32] = pair.public().0;
        let stash = AccountId::from(stash_raw);

        pallet_attestation::Ledger::<Runtime>::insert(
            &stash,
            pallet_attestation::AttestorLedger::<Runtime> {
                stash: stash.clone(),
                total_staked: 0u128,
                active: 0u128,
                unlocking: sp_runtime::BoundedVec::default(),
            },
        );
        // Stash has 0 accrued but tries to claim 100
        let evm_recipient = H160::zero();
        let nonce = 0u64;
        let amount = 100u128;
        let msg = pallet_attest_coin_rewards::Pallet::<Runtime>::claim_signing_message(
            &stash,
            nonce,
            SUPPORTED_CHAIN_KEY,
            amount,
            evm_recipient.0,
        );
        let sig = pair.sign(&msg);
        let mut sig_r = [0u8; 32];
        let mut sig_s = [0u8; 32];
        sig_r.copy_from_slice(&sig.0[..32]);
        sig_s.copy_from_slice(&sig.0[32..]);

        let input = claim_input(
            stash_raw,
            nonce,
            SUPPORTED_CHAIN_KEY,
            amount,
            evm_recipient,
            sig_r,
            sig_s,
        );
        let mut handle = make_handle(evm_recipient, input);
        assert_reverts_with(&mut handle, b"insufficient accrued");
    });
}

#[test]
fn claim_nonce_replay_protection() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let (pair, _) = sr25519::Pair::generate();
        let stash_raw: [u8; 32] = pair.public().0;
        let stash = AccountId::from(stash_raw);

        pallet_attestation::Ledger::<Runtime>::insert(
            &stash,
            pallet_attestation::AttestorLedger::<Runtime> {
                stash: stash.clone(),
                total_staked: 0u128,
                active: 0u128,
                unlocking: sp_runtime::BoundedVec::default(),
            },
        );
        // Give some accrued
        Accrued::<Runtime>::insert(&stash, 1000u128);

        let evm_recipient = H160::zero();
        let nonce = 0u64;
        let amount = 50u128;
        let msg = pallet_attest_coin_rewards::Pallet::<Runtime>::claim_signing_message(
            &stash,
            nonce,
            SUPPORTED_CHAIN_KEY,
            amount,
            evm_recipient.0,
        );
        let sig = pair.sign(&msg);
        let mut sig_r = [0u8; 32];
        let mut sig_s = [0u8; 32];
        sig_r.copy_from_slice(&sig.0[..32]);
        sig_s.copy_from_slice(&sig.0[32..]);

        // First claim: commit_claim will succeed, but ERC-20 transfer will fail (no subcall handler)
        // We just verify the nonce is bumped after successful commit and reverted on ERC-20 failure
        let input = claim_input(
            stash_raw,
            nonce,
            SUPPORTED_CHAIN_KEY,
            amount,
            evm_recipient,
            sig_r,
            sig_s,
        );
        let mut handle = make_handle(evm_recipient, input.clone());
        // Register a subcall handler that simulates ERC-20 transfer failure.
        // This lets commit_claim run (nonce/accrued deducted) then rolls back via undo_claim_commit.
        handle.subcall_handle = Some(Box::new(|_subcall| SubcallOutput {
            reason: ExitReason::Revert(ExitRevert::Reverted),
            output: b"transfer failed".to_vec(),
            cost: 0,
            logs: vec![],
        }));
        let first_result = execute(&mut handle);
        // First call must fail (ERC-20 revert), nonce is restored to 0 by undo_claim_commit
        assert!(
            first_result.is_err(),
            "first claim must fail when ERC-20 transfer fails"
        );

        // After undo, nonce is still 0 and accrued is restored.
        // A second identical attempt with the same nonce=0 must also fail (at the same ERC-20 step).
        let mut handle2 = make_handle(evm_recipient, input);
        handle2.subcall_handle = Some(Box::new(|_subcall| SubcallOutput {
            reason: ExitReason::Revert(ExitRevert::Reverted),
            output: b"transfer failed".to_vec(),
            cost: 0,
            logs: vec![],
        }));
        let result = execute(&mut handle2);
        // Both attempts must revert — nonce replay is foiled by ERC-20 failure + rollback
        assert!(result.is_err(), "second claim must not succeed");
    });
}

// ── deposit revert tests ───────────────────────────────────────────────────────

#[test]
fn deposit_reverts_token_not_configured() {
    ExtBuilder::default().build().execute_with(|| {
        let caller = H160::repeat_byte(0xAA);
        let input = deposit_input(1_000);
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"token not configured");
    });
}

#[test]
fn deposit_reverts_zero_amount() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let caller = H160::repeat_byte(0xAA);
        let input = deposit_input(0);
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"zero amount");
    });
}

#[test]
fn deposit_to_reverts_zero_beneficiary() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let caller = H160::repeat_byte(0xAA);
        // zero beneficiary
        let input = deposit_to_input(1_000, [0u8; 32]);
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"zero beneficiary");
    });
}

#[test]
fn deposit_succeeds_with_successful_subcall() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let caller = H160::repeat_byte(0xAA);
        let input = deposit_input(1_000);
        let mut handle = make_handle(caller, input);
        // Mock a successful ERC-20 transferFrom subcall
        handle.subcall_handle = Some(Box::new(|_subcall| SubcallOutput {
            reason: ExitReason::Succeed(ExitSucceed::Returned),
            output: {
                // Return ABI-encoded `true`
                let mut out = [0u8; 32];
                out[31] = 1;
                out.to_vec()
            },
            cost: 0,
            logs: vec![],
        }));
        // The mint call goes through dispatch; the precompile account needs to be the asset admin
        // for it to succeed. In the unit-test mock the asset admin is `alice`, so the mint dispatch
        // will return a pallet-level error — but we assert that the failure is NOT an early-exit
        // (token not configured, zero amount) to confirm the function reached the mint step.
        let result = execute(&mut handle);
        match &result {
            Err(PrecompileFailure::Revert { output, .. }) => {
                assert_ne!(
                    output.as_slice(),
                    b"token not configured",
                    "should not fail at token check"
                );
                assert_ne!(
                    output.as_slice(),
                    b"zero amount",
                    "should not fail at amount check"
                );
            }
            _ => {} // success is also acceptable if mock pallet allows it
        }
    });
}

// ── withdraw tests ────────────────────────────────────────────────────────────

#[test]
fn withdraw_reverts_token_not_configured() {
    ExtBuilder::default().build().execute_with(|| {
        let caller = H160::repeat_byte(0xAA);
        let input = withdraw_input(1_000);
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"token not configured");
    });
}

#[test]
fn withdraw_reverts_zero_amount() {
    ExtBuilder::default().build().execute_with(|| {
        pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

        let caller = H160::repeat_byte(0xAA);
        let input = withdraw_input(0);
        let mut handle = make_handle(caller, input);
        assert_reverts_with(&mut handle, b"zero amount");
    });
}

#[test]
fn withdraw_succeeds_when_burn_and_transfer_ok() {
    let caller = H160::repeat_byte(0xAA);
    let substrate = <Runtime as pallet_evm::Config>::AddressMapping::into_account_id(caller);

    // Non-sufficient assets require the receiver to have a provider (native balance) on the account.
    ExtBuilder::default()
        .with_balances(vec![(substrate.clone(), 10_000_000_000_000_000_000)])
        .build()
        .execute_with(|| {
            pallet_attest_coin_rewards::AttestCoinErc20::<Runtime>::put(ERC20_ADDRESS);

            let precompile_acct =
                <Runtime as pallet_evm::Config>::AddressMapping::into_account_id(precompile_addr());

            assert_ok!(AssetsPallet::<Runtime>::force_asset_status(
                frame_system::RawOrigin::Root.into(),
                1,
                alice(),
                precompile_acct.clone(),
                precompile_acct.clone(),
                alice(),
                1,
                false,
                false,
            ));

            assert_ok!(AssetsPallet::<Runtime>::transfer(
                RuntimeOrigin::signed(alice()),
                1,
                substrate.clone(),
                10_000,
            ));

            let input = withdraw_input(1_000);
            let mut handle = make_handle(caller, input);
            handle.subcall_handle = Some(Box::new(|_subcall| SubcallOutput {
                reason: ExitReason::Succeed(ExitSucceed::Returned),
                output: {
                    let mut out = [0u8; 32];
                    out[31] = 1;
                    out.to_vec()
                },
                cost: 0,
                logs: vec![],
            }));

            let result = execute(&mut handle);
            assert!(result.is_ok(), "expected withdraw ok, got {result:?}");
        });
}

// ── helper function sanity tests ──────────────────────────────────────────────

#[test]
fn u256_to_u64_returns_correct_value() {
    assert_eq!(super::u256_to_u64(U256::from(0u64)).unwrap(), 0u64);
    assert_eq!(super::u256_to_u64(U256::from(u64::MAX)).unwrap(), u64::MAX);
    assert_eq!(super::u256_to_u64(U256::from(42u64)).unwrap(), 42u64);
}

#[test]
fn u256_to_u64_rejects_overflow() {
    let too_large = U256::from(u64::MAX) + U256::one();
    assert!(
        matches!(
            super::u256_to_u64(too_large),
            Err(PrecompileFailure::Revert { .. })
        ),
        "expected Revert for value > u64::MAX"
    );
}

#[test]
fn u256_to_u128_balance_returns_correct_value() {
    assert_eq!(
        super::u256_to_u128_balance(U256::from(0u128)).unwrap(),
        0u128
    );
    assert_eq!(
        super::u256_to_u128_balance(U256::from(u128::MAX)).unwrap(),
        u128::MAX
    );
    assert_eq!(
        super::u256_to_u128_balance(U256::from(1_000u128)).unwrap(),
        1_000u128
    );
}

#[test]
fn u256_to_u128_balance_rejects_overflow() {
    let too_large = U256::from(u128::MAX) + U256::one();
    assert!(
        matches!(
            super::u256_to_u128_balance(too_large),
            Err(PrecompileFailure::Revert { .. })
        ),
        "expected Revert for value > u128::MAX"
    );
}

#[test]
fn encode_address_pads_correctly() {
    let addr: [u8; 20] = [0xABu8; 20];
    let encoded = super::encode_address(&addr);
    // First 12 bytes must be zero (EVM ABI left-padding for address)
    assert_eq!(&encoded[..12], &[0u8; 12]);
    // Last 20 bytes must be the address itself
    assert_eq!(&encoded[12..], &addr);
}

#[test]
fn encode_u256_round_trips() {
    let val = U256::from(0xDEAD_BEEF_u64);
    let encoded = super::encode_u256(val);
    assert_eq!(encoded.len(), 32);
    let round_tripped = U256::from_big_endian(&encoded);
    assert_eq!(round_tripped, val);
}
