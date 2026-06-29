//! Incoming message-vote validation + counting (confluence §5.3, §6.5).
//!
//! Called inline by the existing block-attestation p2p task for every gossip frame on the
//! `{chain_key}/message-votes/v1` topic — we piggyback on the same swarm, peers, and discovery as
//! block attestation, adding only the second topic. The decision returned maps straight onto a
//! gossipsub `MessageAcceptance`:
//!
//! * **Reject** — undecodable, wrong chain key, bad/forged signature, or a non-attestor signer.
//!   Never propagate.
//! * **Ignore** — valid but redundant (duplicate signer) or not-yet-chain-seen (allowlist miss):
//!   don't count, don't propagate, but don't penalise the sender.
//! * **Accept** — valid, authorized, chain-seen, newly counted. `reached_threshold` is true exactly
//!   on the transition that meets 2N/3+1.

use std::time::Instant;

use alloy::primitives::B256;

use write_ability::envelope::MessageVote;

use super::aggregator::VoteOutcome;
use super::signing::recover_signer;
use super::MessageVoteState;

/// Validation decision for an incoming gossip frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Acceptance {
    Accept {
        reached_threshold: bool,
        message_hash: B256,
    },
    Ignore,
    Reject,
}

/// Validate and (if admissible) count an incoming message-vote frame.
pub fn validate_and_count(
    state: &MessageVoteState,
    our_chain_key: u64,
    bytes: &[u8],
) -> Acceptance {
    let Ok(vote) = MessageVote::decode_bytes(bytes) else {
        tracing::warn!("⛔ undecodable message vote — rejecting");
        return Acceptance::Reject;
    };
    if vote.chain_key != our_chain_key {
        tracing::warn!(
            vote.chain_key,
            "🌐 wrong chain key on message vote — rejecting"
        );
        return Acceptance::Reject;
    }

    let message_hash = B256::from(vote.message_hash);

    // Recover the signer from the signature and require it to match the advertised `signer` field
    // and to be in the active attestor set. Recovering first stops a forged `signer` being trusted.
    let recovered = match recover_signer(&message_hash, &vote.signature) {
        Ok(addr) => addr,
        Err(err) => {
            tracing::warn!(%err, "🔏 unrecoverable message-vote signature — rejecting");
            return Acceptance::Reject;
        }
    };
    if recovered.into_array() != vote.signer {
        tracing::warn!("🔏 message-vote signer mismatch — rejecting");
        return Acceptance::Reject;
    }
    if !state.active_set.read().contains(&recovered) {
        tracing::warn!(signer = %recovered, "👤 message vote from non-attestor — rejecting");
        return Acceptance::Reject;
    }

    // Chain-first allowlist + dedup live in the aggregator.
    match state
        .aggregator
        .lock()
        .add_vote(vote.message_hash, recovered, Instant::now())
    {
        VoteOutcome::Accepted { reached_threshold } => Acceptance::Accept {
            reached_threshold,
            message_hash,
        },
        VoteOutcome::Duplicate => Acceptance::Ignore,
        VoteOutcome::NotIndexed => {
            tracing::debug!(%message_hash, "🚮 vote for unindexed message — dropping");
            Acceptance::Ignore
        }
    }
}

/// Record that a message reached the delivery threshold. Relayers perform the actual on-chain
/// delivery; the attestor only surfaces the milestone for observability.
pub fn note_threshold(chain_key: u64, message_hash: &B256) {
    tracing::info!(
        chain_key,
        %message_hash,
        "🎯 message vote reached 2/3+1 — ready for relayer delivery"
    );
}

#[cfg(test)]
mod tests {
    //! End-to-end validation tests (confluence T1 happy path + T4 abuse): sign with a real key,
    //! then exercise the full decode → recover → allowlist → count path through a `MessageVoteState`.

    use std::collections::HashSet;
    use std::time::{Duration, Instant};

    use parking_lot::{Mutex, RwLock};

    use super::*;
    use crate::tasks::write_ability::aggregator::VoteAggregator;
    use crate::tasks::write_ability::signing::MessageSigner;
    use crate::tasks::write_ability::MessageVoteState;

    const CHAIN_KEY: u64 = 7;

    fn state_with(signers: &[&MessageSigner], threshold: usize) -> MessageVoteState {
        let active_set: HashSet<_> = signers.iter().map(|s| s.address()).collect();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        MessageVoteState {
            aggregator: Mutex::new(VoteAggregator::new(
                threshold,
                1000,
                Duration::from_secs(60),
            )),
            active_set: RwLock::new(active_set),
            publish_tx: tx,
        }
    }

    fn signed_vote(signer: &MessageSigner, message_hash: B256) -> MessageVote {
        let signature = signer.sign(&message_hash).unwrap();
        MessageVote {
            chain_key: CHAIN_KEY,
            message_id: [1u8; 32],
            message_hash: message_hash.0,
            signer: signer.address().into_array(),
            signature,
        }
    }

    #[test]
    fn valid_chain_seen_vote_is_accepted_and_reaches_threshold() {
        let signer = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let state = state_with(&[&signer], 1);
        let hash = B256::from([0xABu8; 32]);
        // Chain-first allowlist: must be indexed before votes count.
        state.aggregator.lock().note_indexed(hash.0, Instant::now());

        let bytes = signed_vote(&signer, hash).encode_bytes();
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, &bytes),
            Acceptance::Accept {
                reached_threshold: true,
                message_hash: hash
            }
        );
    }

    #[test]
    fn vote_for_unindexed_message_is_ignored() {
        let signer = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let state = state_with(&[&signer], 1);
        let hash = B256::from([0xCDu8; 32]);
        // Not indexed → dropped without counting (chain-first allowlist, §5.3).
        let bytes = signed_vote(&signer, hash).encode_bytes();
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, &bytes),
            Acceptance::Ignore
        );
    }

    #[test]
    fn vote_from_non_attestor_is_rejected() {
        let attestor = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let outsider = MessageSigner::from_seed(&[2u8; 32]).unwrap();
        let state = state_with(&[&attestor], 1); // only `attestor` is authorized
        let hash = B256::from([0xABu8; 32]);
        state.aggregator.lock().note_indexed(hash.0, Instant::now());

        let bytes = signed_vote(&outsider, hash).encode_bytes();
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, &bytes),
            Acceptance::Reject
        );
    }

    #[test]
    fn forged_signer_field_is_rejected() {
        let attestor = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let impostor = MessageSigner::from_seed(&[2u8; 32]).unwrap();
        let state = state_with(&[&attestor], 1);
        let hash = B256::from([0xABu8; 32]);
        state.aggregator.lock().note_indexed(hash.0, Instant::now());

        // Sign with the impostor but claim to be the attestor: recovery won't match `signer`.
        let mut vote = signed_vote(&impostor, hash);
        vote.signer = attestor.address().into_array();
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, &vote.encode_bytes()),
            Acceptance::Reject
        );
    }

    #[test]
    fn wrong_chain_key_is_rejected() {
        let signer = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let state = state_with(&[&signer], 1);
        let hash = B256::from([0xABu8; 32]);
        state.aggregator.lock().note_indexed(hash.0, Instant::now());

        let bytes = signed_vote(&signer, hash).encode_bytes();
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY + 1, &bytes),
            Acceptance::Reject
        );
    }

    #[test]
    fn garbage_bytes_are_rejected() {
        let signer = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let state = state_with(&[&signer], 1);
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, b"not a vote"),
            Acceptance::Reject
        );
    }

    #[test]
    fn duplicate_signer_is_ignored_and_does_not_double_count() {
        let s1 = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let s2 = MessageSigner::from_seed(&[2u8; 32]).unwrap();
        let state = state_with(&[&s1, &s2], 2); // threshold 2
        let hash = B256::from([0xABu8; 32]);
        state.aggregator.lock().note_indexed(hash.0, Instant::now());

        let v1 = signed_vote(&s1, hash).encode_bytes();
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, &v1),
            Acceptance::Accept {
                reached_threshold: false,
                message_hash: hash
            }
        );
        // Same signer again → ignored, count stays at 1.
        assert_eq!(
            validate_and_count(&state, CHAIN_KEY, &v1),
            Acceptance::Ignore
        );
        assert_eq!(state.aggregator.lock().signer_count(&hash.0), 1);
    }
}
