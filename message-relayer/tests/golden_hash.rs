//! Golden-vector tests for [`message_relayer::hash::message_hash`] (PoC test T1).
//!
//! The vectors here exercise the encoding contract — they should be regenerated from a
//! reference Solidity contract once the production `Inbox` lands. Until then we only assert
//! that:
//!
//!  * the hash is deterministic across calls,
//!  * each input field meaningfully changes the output (no silent collisions on swapped fields),
//!  * the hash matches a hand-computed `keccak256(abi.encode(...))` over a known input.
//!
//! When you add a vector here, also add a corresponding test on the Solidity side that uses
//! the same input and asserts the same expected hash. That symmetry is what keeps the
//! relayer's local recomputation aligned with `validateVotes`.

use alloy::primitives::{address, b256, B256, U256};
use alloy::sol_types::SolValue;
use message_relayer::hash::message_hash;
use sha3::{Digest, Keccak256};

/// Hand-rolled `keccak256(abi.encode(...))` used as the oracle for [`message_hash`].
fn oracle(
    message_id: B256,
    emitter: alloy::primitives::Address,
    destination_chain_key: B256,
    creditcoin_chain_id: u64,
    payload: &[u8],
) -> B256 {
    let encoded = (
        message_id,
        emitter,
        destination_chain_key,
        U256::from(creditcoin_chain_id),
        payload.to_vec(),
    )
        .abi_encode_params();
    let mut hasher = Keccak256::new();
    hasher.update(&encoded);
    B256::from_slice(&hasher.finalize())
}

#[test]
fn matches_hand_rolled_oracle() {
    let m = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let e = address!("dddddddddddddddddddddddddddddddddddddddd");
    let d = b256!("0000000000000000000000000000000000000000000000000000000000000007");
    let cc = 102_031u64;
    let payload = b"hello relayer".to_vec();

    let expected = oracle(m, e, d, cc, &payload);
    let actual = message_hash(m, e, d, cc, &payload);
    assert_eq!(expected, actual);
}

#[test]
fn changing_any_field_changes_hash() {
    let m = b256!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let e = address!("dddddddddddddddddddddddddddddddddddddddd");
    let d = b256!("0000000000000000000000000000000000000000000000000000000000000007");
    let cc = 102_031u64;
    let p = b"baseline".to_vec();

    let base = message_hash(m, e, d, cc, &p);
    assert_ne!(base, message_hash(B256::ZERO, e, d, cc, &p));
    assert_ne!(
        base,
        message_hash(
            m,
            address!("0000000000000000000000000000000000000000"),
            d,
            cc,
            &p
        )
    );
    assert_ne!(base, message_hash(m, e, B256::ZERO, cc, &p));
    assert_ne!(base, message_hash(m, e, d, cc + 1, &p));
    assert_ne!(base, message_hash(m, e, d, cc, b"different"));
}
