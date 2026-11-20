//! Signing digests shared by the attestor (signer), the relayer (pre-aggregation recovery) and
//! the destination `EOAValidator` (on-chain recovery).
//!
//! * [`MessageVoteDomain::digest`] is the EIP-712 digest every message vote signs (CSUB-2126).
//!   Before it, attestors signed the raw `messageId`, which bound a vote to the source publication
//!   (`OutboxTypes.computeMessageId`) but not to the destination chain, validator or Inbox. The
//!   typed digest adds that binding, and a type hash that keeps message votes apart from the other
//!   raw 32-byte hashes the same attestor key signs (set updates, EVM-address registration).
//! * [`attestor_set_update_digest`] is the raw set-update preimage `submitAttestorSetUpdate`
//!   recomputes.
//!
//! Golden vectors for both digests live in `vectors/`. asc-contracts generates them from
//! Solidity (`npm run vectors`) and the relayer asserts the same files; see `tests/vectors.rs`.

use std::borrow::Cow;

use alloy::primitives::{keccak256, Address, B256, U256};
use alloy::sol_types::{Eip712Domain, SolStruct, SolValue};

/// EIP-712 domain `name` of the destination `EOAValidator`. Must equal the `EIP712(name, version)`
/// constructor arguments on-chain, otherwise every vote recovers to a different address.
pub const MESSAGE_VOTE_DOMAIN_NAME: &str = "ASC EOAValidator";
/// EIP-712 domain `version` of the destination `EOAValidator`.
pub const MESSAGE_VOTE_DOMAIN_VERSION: &str = "1";

mod typed {
    alloy::sol! {
        /// `MessageVote(bytes32 messageId,address inbox)`. `inbox` is the destination Inbox that
        /// calls `validateVotes`; the validator fills it from `msg.sender`, so a vote bundle cannot
        /// be replayed into a second Inbox that happens to accept the same Outbox.
        struct MessageVote {
            bytes32 messageId;
            address inbox;
        }
    }
}

/// Everything a message vote is bound to besides the `messageId` itself: the destination chain,
/// the `EOAValidator` that verifies it (EIP-712 `verifyingContract`) and the Inbox that asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MessageVoteDomain {
    /// Destination chain `block.chainid`.
    pub chain_id: u64,
    /// Destination `EOAValidator` address (EIP-712 `verifyingContract`).
    pub validator: Address,
    /// Destination Inbox that calls `validateVotes` (the `inbox` struct field).
    pub inbox: Address,
}

impl MessageVoteDomain {
    /// The EIP-712 domain, as OZ `EIP712("ASC EOAValidator", "1")` builds it on the validator.
    #[must_use]
    pub fn eip712_domain(&self) -> Eip712Domain {
        Eip712Domain::new(
            Some(Cow::Borrowed(MESSAGE_VOTE_DOMAIN_NAME)),
            Some(Cow::Borrowed(MESSAGE_VOTE_DOMAIN_VERSION)),
            Some(U256::from(self.chain_id)),
            Some(self.validator),
            None,
        )
    }

    /// The digest an attestor signs (raw, no EIP-191 prefix) for `message_id`, and that the
    /// validator recovers against: `keccak256(0x1901 ‖ domainSeparator ‖ hashStruct(MessageVote))`.
    #[must_use]
    pub fn digest(&self, message_id: B256) -> B256 {
        typed::MessageVote {
            messageId: message_id,
            inbox: self.inbox,
        }
        .eip712_signing_hash(&self.eip712_domain())
    }
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

    /// One entry of `vectors/message_vote_eip712.json`.
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Vector {
        chain_id: u64,
        validator: Address,
        inbox: Address,
        message_id: B256,
        digest: B256,
    }

    fn vectors() -> Vec<Vector> {
        serde_json::from_str(include_str!("../vectors/message_vote_eip712.json"))
            .expect("vectors file is valid JSON")
    }

    /// The pinned vectors are the cross-repo contract: the Hardhat suite in asc-contracts and the
    /// relayer assert the same file, so all three sides agree on every byte of the digest.
    #[test]
    fn message_vote_digest_matches_shared_vectors() {
        let vectors = vectors();
        assert!(!vectors.is_empty());
        for v in vectors {
            let domain = MessageVoteDomain {
                chain_id: v.chain_id,
                validator: v.validator,
                inbox: v.inbox,
            };
            assert_eq!(
                domain.digest(v.message_id),
                v.digest,
                "chain {}",
                v.chain_id
            );
        }
    }

    /// Recompute the digest by hand from the EIP-712 spec, independent of alloy's `sol!` derive,
    /// so a wrong type string or field order cannot hide behind the same library on both sides.
    #[test]
    fn message_vote_digest_matches_hand_built_eip712() {
        let domain = MessageVoteDomain {
            chain_id: 11_155_111,
            validator: address!("71a21ea8d28d3a0618d61d478ee20dcb64be8082"),
            inbox: address!("00000000000000000000000000000000000001b0"),
        };
        let message_id = B256::repeat_byte(0x11);

        let domain_typehash = keccak256(
            "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
        );
        let separator = keccak256(
            (
                domain_typehash,
                keccak256(MESSAGE_VOTE_DOMAIN_NAME),
                keccak256(MESSAGE_VOTE_DOMAIN_VERSION),
                U256::from(domain.chain_id),
                domain.validator,
            )
                .abi_encode(),
        );
        let struct_hash = keccak256(
            (
                keccak256("MessageVote(bytes32 messageId,address inbox)"),
                message_id,
                domain.inbox,
            )
                .abi_encode(),
        );
        let mut preimage = vec![0x19, 0x01];
        preimage.extend_from_slice(separator.as_slice());
        preimage.extend_from_slice(struct_hash.as_slice());

        assert_eq!(domain.digest(message_id), keccak256(&preimage));
    }

    #[test]
    fn message_vote_digest_binds_every_domain_field() {
        let base = MessageVoteDomain {
            chain_id: 42,
            validator: address!("00000000000000000000000000000000000000e1"),
            inbox: address!("00000000000000000000000000000000000001b0"),
        };
        let id = B256::repeat_byte(0x22);
        let d = base.digest(id);
        assert_ne!(d, id, "must not sign the raw messageId any more");
        assert_ne!(
            d,
            MessageVoteDomain {
                chain_id: 43,
                ..base
            }
            .digest(id)
        );
        assert_ne!(
            d,
            MessageVoteDomain {
                validator: address!("00000000000000000000000000000000000000e2"),
                ..base
            }
            .digest(id)
        );
        assert_ne!(
            d,
            MessageVoteDomain {
                inbox: address!("00000000000000000000000000000000000001b1"),
                ..base
            }
            .digest(id)
        );
        assert_ne!(d, base.digest(B256::repeat_byte(0x23)));
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
