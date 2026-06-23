//! `MessageVote` — the envelope attesters gossip on `{chain_key}/message-votes/v1`.
//!
//! This is the canonical wire type, shared by the attestor (which signs and publishes votes) and
//! the `message-relayer` (which snoops the topic, counts unique signers, and assembles the inbox
//! calldata). Keeping one definition here is what guarantees the two stay byte-compatible.
//!
//! The on-the-wire encoding is SCALE (`parity_scale_codec`) so it round-trips byte-for-byte across
//! both crates. Do **not** switch to JSON / CBOR here — that would silently desync the mesh.

use parity_scale_codec::{Decode, Encode};

/// Vote envelope as published by attesters and consumed by the relayer.
#[derive(Clone, Debug, Eq, PartialEq, Encode, Decode)]
pub struct MessageVote {
    /// USC chain_key this vote is scoped to. Must match the gossipsub topic prefix.
    pub chain_key: u64,
    /// Outbox `messageId` the attester is voting on.
    pub message_id: [u8; 32],
    /// `keccak256(abi.encode(...))` per PoC §5.2 — the signed digest.
    pub message_hash: [u8; 32],
    /// EVM address recovered from `signature` (also published explicitly so the relayer can
    /// short-circuit the allowlist check before paying for `ecrecover`).
    pub signer: [u8; 20],
    /// 65-byte ECDSA signature (`r || s || v`) over `message_hash` matching the reference
    /// `EOAValidator` (PoC §6.2).
    pub signature: [u8; 65],
}

impl MessageVote {
    /// Decode an incoming gossipsub payload.
    pub fn decode_bytes(bytes: &[u8]) -> Result<Self, parity_scale_codec::Error> {
        Self::decode(&mut &bytes[..])
    }

    /// Encode for publishing / fixtures.
    #[must_use]
    pub fn encode_bytes(&self) -> Vec<u8> {
        self.encode()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> MessageVote {
        MessageVote {
            chain_key: 2,
            message_id: [1u8; 32],
            message_hash: [2u8; 32],
            signer: [3u8; 20],
            signature: [4u8; 65],
        }
    }

    #[test]
    fn round_trips() {
        let v = fixture();
        let bytes = v.encode_bytes();
        let back = MessageVote::decode_bytes(&bytes).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn rejects_truncated() {
        // A truncated frame must fail to decode. We assert on the error *outcome* rather than the
        // codec's error string, which is not part of parity-scale-codec's stable API and has
        // changed wording across versions.
        let bytes = fixture().encode_bytes();
        let truncated = &bytes[..bytes.len() - 4];
        assert!(MessageVote::decode_bytes(truncated).is_err());
    }
}
