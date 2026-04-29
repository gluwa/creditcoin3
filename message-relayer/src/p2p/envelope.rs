//! `MessageVote` — the envelope attesters gossip on `{chain_key}/message-votes/v1`.
//!
//! This crate currently defines its **own** copy of the envelope based on PoC §6.1 / §6.5
//! (the attester write-ability work is paired but not yet merged). When the canonical
//! attester type ships, the swap is one of:
//!
//!  * keep this struct and add `From<canonical>` / `Into<canonical>` impls, or
//!  * delete this struct and re-export the canonical one — preferred.
//!
//! The on-the-wire encoding is SCALE so it round-trips byte-for-byte with the attester crate
//! (which uses `parity_scale_codec` throughout). Avoid switching to JSON / CBOR here — that
//! would silently desync from attesters.

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

    /// Encode for tests / fixtures (relayers do not publish in PoC scope).
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
        let bytes = fixture().encode_bytes();
        let truncated = &bytes[..bytes.len() - 4];
        let err = MessageVote::decode_bytes(truncated).unwrap_err();
        assert!(format!("{err:?}").to_lowercase().contains("not enough"));
    }
}
