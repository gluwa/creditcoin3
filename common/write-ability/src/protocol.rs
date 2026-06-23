//! Shared protocol constants for USC message-vote gossip.
//!
//! The gossipsub topic and the `u64 → bytes32` chain-key encoding both have to agree across the
//! attestor (publisher), the relayer (subscriber), and the on-chain Outbox/factory. Defining them
//! once here is what keeps the mesh and the `getOutbox(bytes32)` lookups consistent.

use alloy::primitives::B256;

/// Build the gossipsub topic attesters publish message votes on for a given USC `chain_key`, and
/// the relayer subscribes to. Distinct from the block-attestation topic `{chain_key}/attest`
/// (PoC §4, §6.1).
#[must_use]
pub fn message_votes_topic(chain_key: u64) -> String {
    format!("{chain_key}/message-votes/v1")
}

/// Canonical mapping of a Substrate `ChainKey` (`u64`) to the Solidity `bytes32` chain key passed
/// to `IOutboxFactory.getOutbox` and asserted against `Outbox.chainKey()` (research §2.3).
///
/// The `u64` is stored big-endian in the **low** 8 bytes (left-padded with zeros), i.e.
/// `bytes32(uint256(value))` in Solidity terms.
#[must_use]
pub fn chain_key_to_bytes32(value: u64) -> B256 {
    let mut bytes = [0u8; 32];
    bytes[24..32].copy_from_slice(&value.to_be_bytes());
    B256::from(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topic_matches_spec() {
        assert_eq!(message_votes_topic(42), "42/message-votes/v1");
    }

    #[test]
    fn chain_key_encoding_is_low_eight_bytes_big_endian() {
        let b = chain_key_to_bytes32(0x0102_0304_0506_0708);
        // High 24 bytes zero, low 8 bytes are the big-endian u64.
        assert_eq!(&b.as_slice()[..24], &[0u8; 24]);
        assert_eq!(
            &b.as_slice()[24..],
            &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]
        );
    }

    #[test]
    fn chain_key_zero_is_all_zero() {
        assert_eq!(chain_key_to_bytes32(0), B256::ZERO);
    }
}
