use crate::mock::*;
use precompile_utils::testing::*;
use sp_core::{ed25519, Pair, H160, H256};

const THREE_MB: usize = 3 * 1024 * 1024; // 3,145,728 bytes

fn precompiles() -> Precompiles<Runtime> {
    PrecompilesValue::get()
}

#[test]
fn verify_valid_signature_should_return_true() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();
        let message = b"Hello!";

        // Sign the message
        let signature = pair.sign(message);

        let caller: H160 = Account::Alice.into();

        // Call the precompile
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.to_vec().into(),
                    signature: signature.0.to_vec().into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_returns(true);
    });
}

#[test]
fn verify_bogus_signature_should_return_false() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();
        let message = b"Hello!";

        // create a bogus signature
        let bogus_signature = vec![0u8; 64];

        let caller: H160 = Account::Alice.into();

        // Call the precompile
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.to_vec().into(),
                    signature: bogus_signature.into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_returns(false);
    });
}

#[test]
fn verify_tampered_message_should_return_false() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();
        let message = b"Hello!";

        // Sign the message
        let signature = pair.sign(message);

        // Modify the message
        let tampered_message = b"Hello, World!";

        let caller: H160 = Account::Alice.into();

        // Call the precompile with tampered message
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: tampered_message.to_vec().into(),
                    signature: signature.0.to_vec().into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_returns(false);
    });
}

#[test]
fn verify_wrong_public_key_should_return_false() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate two keypairs
        let (pair1, _seed1) = ed25519::Pair::generate();
        let (pair2, _seed2) = ed25519::Pair::generate();
        let public2 = pair2.public();

        let message = b"Hello!";

        // Sign with pair1
        let signature = pair1.sign(message);

        let caller: H160 = Account::Alice.into();

        // Verify with pair2's public key
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.to_vec().into(),
                    signature: signature.0.to_vec().into(),
                    public_key: H256::from(public2.0),
                },
            )
            .expect_no_logs()
            .execute_returns(false);
    });
}

#[test]
fn verify_invalid_signature_length_should_revert() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();
        let message = b"Hello!";

        // Create an invalid signature (wrong length)
        let invalid_signature = vec![0u8; 32]; // Should be 64 bytes

        let caller: H160 = Account::Alice.into();

        // Call the precompile with invalid signature length
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.to_vec().into(),
                    signature: invalid_signature.into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_reverts(|output| {
                output == b"Invalid signature length: must be exactly 64 bytes"
            });
    });
}

#[test]
fn verify_empty_message_should_work() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();
        let message = b"";

        // Sign the empty message
        let signature = pair.sign(message);

        let caller: H160 = Account::Alice.into();

        // Call the precompile
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.to_vec().into(),
                    signature: signature.0.to_vec().into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_returns(true);
    });
}

#[test]
fn verify_long_message_should_work() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();

        // Create a long message
        let message = vec![0x42u8; 10000];

        // Sign the long message
        let signature = pair.sign(&message);

        let caller: H160 = Account::Alice.into();

        // Call the precompile
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.into(),
                    signature: signature.0.to_vec().into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_returns(true);
    });
}

#[test]
fn verify_signature_too_long_should_revert() {
    ExtBuilder::default().build().execute_with(|| {
        // Generate a keypair
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();
        let message = b"Hello!";

        // Create an invalid signature (too long)
        let invalid_signature = vec![0u8; 100]; // Should be exactly 64 bytes

        let caller: H160 = Account::Alice.into();

        // Call the precompile with invalid signature length
        // This will be caught by BoundedBytes boundary check before reaching our code
        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.to_vec().into(),
                    signature: invalid_signature.into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_reverts(|output| output == b"signature: Value is too large for length");
    });
}

#[test]
fn verify_message_exceeding_3mb_should_revert() {
    ExtBuilder::default().build().execute_with(|| {
        let (pair, _seed) = ed25519::Pair::generate();
        let public = pair.public();

        // 3MB + 1 byte exceeds the BoundedBytes<ConstU3MB> limit
        let message = vec![0x42u8; THREE_MB + 1];
        let signature = pair.sign(&message);

        let caller: H160 = Account::Alice.into();

        precompiles()
            .prepare_test(
                caller,
                Account::Precompile,
                PCall::verify {
                    message: message.into(),
                    signature: signature.0.to_vec().into(),
                    public_key: H256::from(public.0),
                },
            )
            .expect_no_logs()
            .execute_reverts(|output| output == b"message: Value is too large for length");
    });
}
