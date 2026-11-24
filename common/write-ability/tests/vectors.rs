//! Cross-repo golden vectors.
//!
//! * The files generated from Solidity (`npm run vectors` in asc-contracts, their canonical home)
//!   are copied here byte for byte. This crate asserts the ones it implements; the TypeScript
//!   tooling under `usc-messaging/` asserts the rest from the same copy.
//! * The SCALE gossip envelopes have no Solidity counterpart, so this crate is their canonical
//!   home: `UPDATE_VECTORS=1 cargo test -p write-ability --test vectors` regenerates them and a
//!   plain run fails on any difference. asc-message-relayer copies and asserts them.
//!
//! Changing an existing entry changes a wire or signing format that deployed attestors and
//! relayers depend on. Add entries instead.

use std::path::{Path, PathBuf};

use alloy::primitives::{Address, Bytes, B256, U256};
use serde::{Deserialize, Serialize};
use write_ability::envelope::{MessageVote, ReobservationRequest, SetUpdateVote};
use write_ability::hash::{attestor_set_update_digest, canonical_attestor_order};

/// Files whose source of truth is asc-contracts `test/vectors/`.
const SOLIDITY_VECTORS: [&str; 4] = [
    "attestor_set_update.json",
    "evm_envelope.json",
    "message_id.json",
    "message_vote_eip712.json",
];

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors")
}

fn decimal(value: &str) -> U256 {
    U256::from_str_radix(value, 10).expect("vector integers are decimal strings")
}

/// One entry of `vectors/attestor_set_update.json`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SetUpdate {
    name: String,
    validator: Address,
    attestors: Vec<Address>,
    chain_id: String,
    nonce: String,
    digest: B256,
    canonical_attestors: Vec<Address>,
    canonical_digest: B256,
}

/// The digest `EOAValidator.submitAttestorSetUpdate` recovers signers over, and the ordering every
/// attestor applies before signing, against what Solidity produced.
#[test]
fn attestor_set_update_digest_matches_solidity_vectors() {
    let vectors: Vec<SetUpdate> =
        serde_json::from_str(include_str!("../vectors/attestor_set_update.json"))
            .expect("vectors file is valid JSON");
    assert!(!vectors.is_empty());
    for v in vectors {
        let (chain_id, nonce) = (decimal(&v.chain_id), decimal(&v.nonce));
        assert_eq!(
            attestor_set_update_digest(v.validator, &v.attestors, chain_id, nonce),
            v.digest,
            "{}: digest over the given order",
            v.name
        );
        let canonical = canonical_attestor_order(&v.attestors);
        assert_eq!(
            canonical, v.canonical_attestors,
            "{}: canonical order",
            v.name
        );
        assert_eq!(
            attestor_set_update_digest(v.validator, &canonical, chain_id, nonce),
            v.canonical_digest,
            "{}: digest over the canonical order",
            v.name
        );
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessageVoteVector {
    name: &'static str,
    chain_key: String,
    message_id: B256,
    signer: Address,
    signature: Bytes,
    encoded: Bytes,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReobservationRequestVector {
    name: &'static str,
    chain_key: String,
    message_id: B256,
    tx_hash: B256,
    block_height: String,
    encoded: Bytes,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SetUpdateVoteVector {
    name: &'static str,
    chain_key: String,
    new_attestors: Vec<Address>,
    nonce: B256,
    signer: Address,
    signature: Bytes,
    encoded: Bytes,
}

/// Distinct bytes in every position, so a swapped or shifted field cannot encode to the same frame.
fn counting<const N: usize>(start: u8) -> [u8; N] {
    std::array::from_fn(|i| start.wrapping_add(i as u8))
}

fn message_votes() -> Vec<(&'static str, MessageVote)> {
    vec![
        (
            "all-zero",
            MessageVote {
                chain_key: 0,
                message_id: [0; 32],
                signer: [0; 20],
                signature: [0; 65],
            },
        ),
        (
            "all-max",
            MessageVote {
                chain_key: u64::MAX,
                message_id: [0xff; 32],
                signer: [0xff; 20],
                signature: [0xff; 65],
            },
        ),
        (
            "distinct-bytes",
            MessageVote {
                chain_key: 2,
                message_id: counting(0x00),
                signer: counting(0x40),
                signature: counting(0x80),
            },
        ),
    ]
}

fn reobservation_requests() -> Vec<(&'static str, ReobservationRequest)> {
    vec![
        (
            "all-zero",
            ReobservationRequest {
                chain_key: 0,
                message_id: [0; 32],
                tx_hash: [0; 32],
                block_height: 0,
            },
        ),
        (
            "all-max",
            ReobservationRequest {
                chain_key: u64::MAX,
                message_id: [0xff; 32],
                tx_hash: [0xff; 32],
                block_height: u64::MAX,
            },
        ),
        (
            "distinct-bytes",
            ReobservationRequest {
                chain_key: 2,
                message_id: counting(0x00),
                tx_hash: counting(0x40),
                block_height: 0x0102_0304_0506_0708,
            },
        ),
    ]
}

fn set_update_votes() -> Vec<(&'static str, SetUpdateVote)> {
    let vote = |chain_key, new_attestors, nonce| SetUpdateVote {
        chain_key,
        new_attestors,
        nonce,
        signer: counting(0x40),
        signature: counting(0x80),
    };
    vec![
        ("empty-set", vote(0, vec![], [0; 32])),
        (
            "single-attestor",
            vote(2, vec![counting(0x01)], counting(0)),
        ),
        (
            "four-attestors",
            vote(
                2,
                (0..4).map(|i| counting(0x10 * (i + 1))).collect(),
                U256::from(7u64).to_be_bytes(),
            ),
        ),
        // 64 entries is the first length whose SCALE compact prefix takes two bytes.
        (
            "two-byte-length-prefix",
            vote(u64::MAX, (0..64).map(counting).collect(), [0xff; 32]),
        ),
    ]
}

/// Compares `file` with the vectors this crate generates, or rewrites it under `UPDATE_VECTORS`.
fn assert_generated<T: Serialize>(file: &str, vectors: &[T]) {
    let path = vectors_dir().join(file);
    let expected = serde_json::to_string_pretty(vectors).expect("vectors serialize") + "\n";
    if std::env::var("UPDATE_VECTORS").is_ok() {
        std::fs::write(&path, expected).expect("vectors file is writable");
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == expected,
        "{file} differs from the SCALE encoding this crate produces. If the wire change is \
         intended, rerun with UPDATE_VECTORS=1 and update asc-message-relayer's copy"
    );
}

#[test]
fn message_vote_scale_vectors_are_current() {
    let vectors: Vec<_> = message_votes()
        .into_iter()
        .map(|(name, vote)| {
            let encoded = vote.encode_bytes();
            assert_eq!(MessageVote::decode_bytes(&encoded).as_ref(), Ok(&vote));
            MessageVoteVector {
                name,
                chain_key: vote.chain_key.to_string(),
                message_id: vote.message_id.into(),
                signer: vote.signer.into(),
                signature: vote.signature.to_vec().into(),
                encoded: encoded.into(),
            }
        })
        .collect();
    assert_generated("message_vote_scale.json", &vectors);
}

#[test]
fn reobservation_request_scale_vectors_are_current() {
    let vectors: Vec<_> = reobservation_requests()
        .into_iter()
        .map(|(name, request)| {
            let encoded = request.encode_bytes();
            assert_eq!(
                ReobservationRequest::decode_bytes(&encoded).as_ref(),
                Ok(&request)
            );
            ReobservationRequestVector {
                name,
                chain_key: request.chain_key.to_string(),
                message_id: request.message_id.into(),
                tx_hash: request.tx_hash.into(),
                block_height: request.block_height.to_string(),
                encoded: encoded.into(),
            }
        })
        .collect();
    assert_generated("reobservation_request_scale.json", &vectors);
}

#[test]
fn set_update_vote_scale_vectors_are_current() {
    let vectors: Vec<_> = set_update_votes()
        .into_iter()
        .map(|(name, vote)| {
            let encoded = vote.encode_bytes();
            assert_eq!(SetUpdateVote::decode_bytes(&encoded).as_ref(), Ok(&vote));
            SetUpdateVoteVector {
                name,
                chain_key: vote.chain_key.to_string(),
                new_attestors: vote.new_attestors.iter().copied().map(Into::into).collect(),
                nonce: vote.nonce.into(),
                signer: vote.signer.into(),
                signature: vote.signature.to_vec().into(),
                encoded: encoded.into(),
            }
        })
        .collect();
    assert_generated("set_update_vote_scale.json", &vectors);
}

/// The Solidity-generated files must be the exact bytes asc-contracts holds, so a local edit or a
/// stale copy cannot keep the tests above green against a format the contracts no longer use.
#[test]
fn solidity_vectors_match_asc_contracts() {
    // Same switches as the ABI-surface gate: an unset directory skips on a dev machine and fails
    // where `ABI_GATE_STRICT` says the gate is meant to be enforced.
    let strict = std::env::var("ABI_GATE_STRICT").is_ok();
    let Ok(dir) = std::env::var("ASC_CONTRACTS_DIR") else {
        assert!(
            !strict,
            "ABI_GATE_STRICT is set but ASC_CONTRACTS_DIR is not; the vector drift gate would \
             have passed without checking anything"
        );
        eprintln!("ASC_CONTRACTS_DIR not set; skipping the shared-vector drift check");
        return;
    };
    let canonical = Path::new(&dir).join("test/vectors");
    for file in SOLIDITY_VECTORS {
        let theirs = std::fs::read(canonical.join(file)).unwrap_or_else(|error| {
            panic!(
                "{}: {error}. The pinned asc-contracts revision must contain the shared vectors",
                canonical.join(file).display()
            )
        });
        let ours = std::fs::read(vectors_dir().join(file)).expect("local vector copy exists");
        assert!(
            ours == theirs,
            "vectors/{file} differs from asc-contracts test/vectors/{file}; copy the canonical \
             file over and fix whichever implementation it exposes"
        );
    }
}
