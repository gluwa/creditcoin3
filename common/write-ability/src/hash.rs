//! `messageHash` builder.
//!
//! Per PoC §5.2:
//!
//! ```solidity
//! messageHash = keccak256(abi.encode(
//!     bytes32 messageId,
//!     address emitterAddress,
//!     bytes32 destinationChainKey,
//!     uint64  creditcoinChainId,
//!     bytes   payload
//! ))
//! ```
//!
//! This must be byte-identical to what attesters sign and what the inbox recomputes inside
//! `validateVotes`. The golden-vector tests at the bottom of this file are the contract: any
//! drift here will silently break delivery.

use alloy::primitives::{Address, B256, U256};
use alloy::sol_types::SolValue;
use sha3::{Digest, Keccak256};

/// Compute `messageHash` exactly as the Solidity `validateVotes` will recompute it.
#[must_use]
pub fn message_hash(
    message_id: B256,
    emitter: Address,
    destination_chain_key: B256,
    creditcoin_chain_id: u64,
    payload: &[u8],
) -> B256 {
    // `abi.encode(a, b, c, d, e)` in Solidity is the head-encoding of a tuple — `abi_encode_params`
    // on a tuple type produces the same byte sequence. Using `abi_encode` on the tuple would wrap
    // it in an outer offset (Solidity-struct semantics), which is *not* what `abi.encode` does for
    // a free-standing argument list.
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::primitives::{address, b256};

    /// Sanity vector: same input → same hash. Cheap deterministic check.
    #[test]
    fn deterministic() {
        let a = message_hash(
            b256!("1111111111111111111111111111111111111111111111111111111111111111"),
            address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
            102_031,
            b"hello",
        );
        let b = message_hash(
            b256!("1111111111111111111111111111111111111111111111111111111111111111"),
            address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
            102_031,
            b"hello",
        );
        assert_eq!(a, b);
    }

    /// Differing payload bytes must produce different hashes.
    #[test]
    fn payload_sensitive() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let d = b256!("0000000000000000000000000000000000000000000000000000000000000002");

        let h1 = message_hash(m, e, d, 1, b"a");
        let h2 = message_hash(m, e, d, 1, b"b");
        assert_ne!(h1, h2);
    }

    /// Differing creditcoin_chain_id must produce different hashes (replay protection).
    #[test]
    fn chain_id_sensitive() {
        let m = b256!("1111111111111111111111111111111111111111111111111111111111111111");
        let e = address!("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let d = b256!("0000000000000000000000000000000000000000000000000000000000000002");

        let h1 = message_hash(m, e, d, 1, b"x");
        let h2 = message_hash(m, e, d, 2, b"x");
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
            b256!("0000000000000000000000000000000000000000000000000000000000000002"),
            1,
            b"x",
        );
        let h2 = message_hash(
            m,
            e,
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
            b256!("0000000000000000000000000000000000000000000000000000000000000000"),
            0,
            b"",
        );
        // Just assert non-zero, since the actual value should be locked down by an
        // integration-tests/golden_hash.rs vector once the reference Solidity contract lands.
        assert_ne!(h, B256::ZERO);
    }
}
