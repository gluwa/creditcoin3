//! `messageHash` builder.
//!
//! Mirrors `Inbox.deliverMessage` on asc-contracts `feature/SMC-1646` (PR #45, merged 2026-09-04):
//!
//! ```solidity
//! messageHash = keccak256(abi.encode(
//!     bytes32 messageId,
//!     address emitterAddress,
//!     address outbox,             // the source Outbox that published the message (#45)
//!     bytes32 localChainKey,      // the destination chain key
//!     uint256 sourceChainId,      // Creditcoin's eth_chainId
//!     bytes   messagePayload
//! ))
//! ```
//!
//! `outbox` was added so a vote for a message on a retired or rogue Outbox can never be replayed as
//! if the canonical one had published it; the Inbox also refuses `deliverMessage` for any Outbox
//! that is not on its allowlist. The field sits after `emitterAddress` so all static words come
//! first and the one dynamic field last. (The order is a style choice: this is `abi.encode`, whose
//! head/tail layout makes every order unambiguous. Only `encodePacked` has the adjacent-dynamic
//! collision problem.)
//!
//! This must be byte-identical to what attestors sign and what the inbox recomputes inside
//! `validateVotes`. The golden vectors at the bottom of this file were produced with Foundry
//! (`cast abi-encode` + `cast keccak`) and are duplicated in asc-message-relayer; any drift here
//! silently breaks delivery.

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::sol_types::SolValue;

/// Compute `messageHash` exactly as the Solidity `validateVotes` will recompute it.
///
/// `outbox` is the Outbox the event was scanned from; the attestor knows it because it resolved
/// that address before listening, and the relayer knows it for the same reason.
#[must_use]
pub fn message_hash(
    message_id: B256,
    emitter: Address,
    outbox: Address,
    destination_chain_key: B256,
    creditcoin_chain_id: u64,
    payload: &[u8],
) -> B256 {
    // `abi.encode(a, b, c, d, e, f)` in Solidity is the head-encoding of a tuple — `abi_encode_params`
    // on a tuple type produces the same byte sequence. Using `abi_encode` on the tuple would wrap
    // it in an outer offset (Solidity-struct semantics), which is *not* what `abi.encode` does for
    // a free-standing argument list.
    let encoded = (
        message_id,
        emitter,
        outbox,
        destination_chain_key,
        U256::from(creditcoin_chain_id),
        payload.to_vec(),
    )
        .abi_encode_params();

    keccak256(&encoded)
}

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
    // Same head-of-tuple encoding as `message_hash`: `abi_encode_params` on the tuple reproduces
    // Solidity `abi.encode(address, address[], uint256, uint256)` byte-for-byte.
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
    use alloy::primitives::{address, b256};

    const OUTBOX: Address = address!("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

    /// Sanity vector: same input → same hash. Cheap deterministic check.
    #[test]
    fn deterministic() {
        let a = message_hash(
            b256!("1111111111111111111111111111111111111111111111111111111111111111"),
            address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            OUTBOX,
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
            102_031,
            b"hello",
        );
        let b = message_hash(
            b256!("1111111111111111111111111111111111111111111111111111111111111111"),
            address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            OUTBOX,
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
            102_031,
            b"hello",
        );
        assert_eq!(a, b);
    }

    /// Golden vectors produced with Foundry against the #45 preimage:
    /// `cast abi-encode 'f(bytes32,address,address,bytes32,uint256,bytes)' …` piped into
    /// `cast keccak`. The same three constants live in asc-message-relayer's golden test; if
    /// either side drifts from the Inbox, attestor signatures stop verifying on-chain.
    #[test]
    fn golden_vectors_match_cast_abi_encode() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("2222222222222222222222222222222222222222");
        let o = address!("3333333333333333333333333333333333333333");
        let d = b256!("0000000000000000000000000000000000000000000000000000000000000008");
        let cc = 42u64;

        assert_eq!(
            message_hash(m, e, o, d, cc, &[0xde, 0xad, 0xbe, 0xef]),
            b256!("4b387cab474082acc4b1765537d74b87b4425bf4ecd4e375472106d63e12ff68"),
            "six-field preimage with a 4-byte payload"
        );
        assert_eq!(
            message_hash(m, e, o, d, cc, b""),
            b256!("a5f139b554b2264af26a30ad40276a909055ca73db27a4e4d005baa1a3f8d013"),
            "six-field preimage with an empty payload"
        );
        // The pre-#45 five-field hash of the same inputs. Must NOT match: this is the vote-format
        // fork the coordinated attestor + relayer + Inbox redeploy exists for.
        assert_ne!(
            message_hash(m, e, o, d, cc, &[0xde, 0xad, 0xbe, 0xef]),
            b256!("c99b69d3aef00fc024cdb4751aae2a8ea1467501e2fdd2824dd415a120deb5bd"),
            "must not collide with the legacy five-field preimage"
        );
    }

    /// Differing Outbox must produce different hashes (no cross-Outbox replay after rotation).
    #[test]
    fn outbox_sensitive() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let d = b256!("0000000000000000000000000000000000000000000000000000000000000002");

        let h1 = message_hash(m, e, OUTBOX, d, 1, b"x");
        let h2 = message_hash(
            m,
            e,
            address!("cccccccccccccccccccccccccccccccccccccccc"),
            d,
            1,
            b"x",
        );
        assert_ne!(h1, h2);
    }

    /// Differing payload bytes must produce different hashes.
    #[test]
    fn payload_sensitive() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let d = b256!("0000000000000000000000000000000000000000000000000000000000000002");

        let h1 = message_hash(m, e, OUTBOX, d, 1, b"a");
        let h2 = message_hash(m, e, OUTBOX, d, 1, b"b");
        assert_ne!(h1, h2);
    }

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

    /// Differing creditcoin_chain_id must produce different hashes (replay protection).
    #[test]
    fn chain_id_sensitive() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let d = b256!("0000000000000000000000000000000000000000000000000000000000000002");

        let h1 = message_hash(m, e, OUTBOX, d, 1, b"x");
        let h2 = message_hash(m, e, OUTBOX, d, 2, b"x");
        assert_ne!(h1, h2);
    }

    /// Differing destination_chain_key must produce different hashes (cross-chain isolation).
    #[test]
    fn destination_key_sensitive() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");

        let h1 = message_hash(
            m,
            e,
            OUTBOX,
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
            1,
            b"x",
        );
        let h2 = message_hash(
            m,
            e,
            OUTBOX,
            b256!("0000000000000000000000000000000000000000000000000000000000000007"),
            1,
            b"x",
        );
        assert_ne!(h1, h2);
    }

    /// Empty payload still produces a defined hash — used by the inbox for control messages.
    #[test]
    fn empty_payload() {
        let h = message_hash(
            b256!("0000000000000000000000000000000000000000000000000000000000000000"),
            address!("0000000000000000000000000000000000000000"),
            address!("0000000000000000000000000000000000000000"),
            b256!("0000000000000000000000000000000000000000000000000000000000000000"),
            0,
            b"",
        );
        // Non-zero here; the exact value is pinned by `golden_vectors_match_cast_abi_encode`.
        assert_ne!(h, B256::ZERO);
    }
}
