//! Attestor-set-update digest builder.
//!
//! Prior to asc-contracts #54, this module also built the vote digest attestors signed over each
//! `MessagePublished` (a six-field `messageHash` binding messageId/emitter/outbox/chain-keys/
//! payload). #54 replaced that with a direct check inside `Inbox.deliverMessage`:
//! `messageId == keccak256(abi.encode(outbox, emitter, sequence, keccak256(payload),
//! sourceChainId))` (see `OutboxTypes.computeMessageId`), and `validateVotes` now takes
//! `messageId` itself as the signed digest. Since `messageId` is already an indexed field on the
//! finalized `MessagePublished` log the attestor observes (or, for reobservation, independently
//! re-fetches from its own RPC), there is nothing left to derive here — attestors sign the
//! `messageId` straight off the event. See `listener.rs` / `reobservation.rs` in the attestor
//! crate.

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::sol_types::SolValue;

/// Compute the attestor-set-update digest exactly as the `EOAValidator` recomputes it:
/// `keccak256(abi.encode(address(this), newAttestors, chainId, nonce))`.
///
/// `validator` is the destination `EOAValidator` the update targets. The contract binds its own
/// address into the preimage, so an update signed for one validator instance cannot be replayed
/// against another instance on the same chain — and since instances share an `AttestorRegistry`,
/// overlapping signer sets at the same nonce are the norm rather than the exception. Omitting it
/// here (as the pre-registry contract did) makes every signature the fleet produces unverifiable:
/// the contract recovers over a different preimage and rejects the whole batch.
///
/// `new_attestors` MUST be in the exact order the relayer will submit on-chain (the contract hashes
/// that order), so every attestor and the relayer agree on a **canonical** ordering — see
/// `canonical_attestor_order`. `chain_id` is the destination chain's `block.chainid`, and `nonce`
/// is the validator's current `attestorSetUpdateNonce` (replay/rollback protection).
#[must_use]
pub fn attestor_set_update_digest(
    validator: Address,
    new_attestors: &[Address],
    chain_id: U256,
    nonce: U256,
) -> B256 {
    // `abi_encode_params` on a tuple type reproduces Solidity's free-standing-argument-list
    // `abi.encode(address, address[], uint256, uint256)` byte-for-byte (no outer struct offset).
    let encoded = (validator, new_attestors.to_vec(), chain_id, nonce).abi_encode_params();
    keccak256(&encoded)
}

/// Canonical ordering for the attestor-set-update array: ascending by 20-byte address. All attestors
/// (and the relayer) must order `newAttestors` identically or their signatures cover different bytes
/// and cannot be aggregated. Sorting by the raw address bytes is deterministic and needs no shared
/// state. Returns a de-duplicated, sorted copy.
#[must_use]
pub fn canonical_attestor_order(addrs: &[Address]) -> Vec<Address> {
    let mut out = addrs.to_vec();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::address;

    /// Golden vector: the digest must equal Solidity
    /// `keccak256(abi.encode(address(this), newAttestors, block.chainid, attestorSetUpdateNonce))`
    /// as `EOAValidator.submitAttestorSetUpdate` computes it. Pinned by hand-encoding the same
    /// preimage here — if either side's field order or types drift, the fleet's signatures stop
    /// verifying on-chain and every set update fails, so the contract shape is worth nailing down
    /// in a test rather than a comment.
    #[test]
    fn set_update_digest_matches_hand_encoded_solidity_preimage() {
        use alloy::primitives::keccak256;

        let validator = address!("71a21ea8d28d3a0618d61d478ee20dcb64be8082");
        let attestors = [
            address!("00000000000000000000000000000000000000aa"),
            address!("00000000000000000000000000000000000000bb"),
        ];
        let chain_id = U256::from(11_155_111u64); // Sepolia
        let nonce = U256::from(3u64);

        // abi.encode(address, address[], uint256, uint256):
        //   head: validator | offset-to-array (0x80) | chain_id | nonce
        //   tail: array length | element 0 | element 1
        let mut expected = Vec::new();
        expected.extend_from_slice(&validator.into_word()[..]);
        expected.extend_from_slice(&U256::from(0x80u64).to_be_bytes::<32>());
        expected.extend_from_slice(&chain_id.to_be_bytes::<32>());
        expected.extend_from_slice(&nonce.to_be_bytes::<32>());
        expected.extend_from_slice(&U256::from(attestors.len()).to_be_bytes::<32>());
        for a in &attestors {
            expected.extend_from_slice(&a.into_word()[..]);
        }

        assert_eq!(
            attestor_set_update_digest(validator, &attestors, chain_id, nonce),
            keccak256(&expected),
            "digest no longer matches abi.encode(address, address[], uint256, uint256)"
        );
    }

    #[test]
    fn set_update_digest_is_deterministic_and_binds_validator_nonce_and_chain() {
        let addrs = [
            address!("00000000000000000000000000000000000000aa"),
            address!("00000000000000000000000000000000000000bb"),
        ];
        let validator = address!("00000000000000000000000000000000000000e1");
        let base =
            attestor_set_update_digest(validator, &addrs, U256::from(42u64), U256::from(7u64));
        // Deterministic.
        assert_eq!(
            base,
            attestor_set_update_digest(validator, &addrs, U256::from(42u64), U256::from(7u64))
        );
        // Validator-sensitive (no cross-instance replay on the same chain).
        let other_validator = address!("00000000000000000000000000000000000000e2");
        assert_ne!(
            base,
            attestor_set_update_digest(
                other_validator,
                &addrs,
                U256::from(42u64),
                U256::from(7u64)
            )
        );
        // Nonce-sensitive (rollback protection).
        assert_ne!(
            base,
            attestor_set_update_digest(validator, &addrs, U256::from(42u64), U256::from(8u64))
        );
        // Chain-id-sensitive (cross-chain isolation).
        assert_ne!(
            base,
            attestor_set_update_digest(validator, &addrs, U256::from(43u64), U256::from(7u64))
        );
        // Order-sensitive (why canonical ordering is mandatory).
        let reversed = [addrs[1], addrs[0]];
        assert_ne!(
            base,
            attestor_set_update_digest(validator, &reversed, U256::from(42u64), U256::from(7u64))
        );
    }

    #[test]
    fn canonical_order_sorts_and_dedups() {
        let a = address!("00000000000000000000000000000000000000aa");
        let b = address!("00000000000000000000000000000000000000bb");
        let c = address!("00000000000000000000000000000000000000cc");
        let ordered = canonical_attestor_order(&[c, a, b, a]);
        assert_eq!(ordered, vec![a, b, c]);
        // Idempotent + order-independent: any permutation yields the same canonical vector.
        assert_eq!(canonical_attestor_order(&[b, c, a]), ordered);
    }
}
