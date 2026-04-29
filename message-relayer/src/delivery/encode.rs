//! `votes` calldata encoder.
//!
//! Produces the byte sequence the inbox expects as the `votes` argument of `deliverMessage`,
//! matching PoC §6.4:
//!
//! ```solidity
//! votes = abi.encode(bytes[] memory signatures);
//! ```
//!
//! Each signature is 65 bytes (`r || s || v`) and signers must already be deduped (the pool
//! enforces that). Sorting by signer address is recommended for deterministic transactions
//! and easier diffing in tests, but the encoding itself does not depend on order.

use alloy::primitives::Bytes;
use alloy::sol_types::SolValue;

/// Encode a list of 65-byte ECDSA signatures as `abi.encode(bytes[] memory)` per PoC §6.4.
#[must_use]
pub fn encode_votes(signatures: &[[u8; 65]]) -> Vec<u8> {
    let sigs: Vec<Bytes> = signatures
        .iter()
        .map(|s| Bytes::copy_from_slice(s))
        .collect();
    // `abi.encode(arg)` with a single dynamic argument → `abi_encode_params` on a one-tuple.
    // Plain `abi_encode()` on `Vec<Bytes>` would give the *array* encoding without the outer
    // offset that Solidity's `abi.encode` adds for top-level dynamic arguments.
    (sigs,).abi_encode_params()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(byte: u8) -> [u8; 65] {
        [byte; 65]
    }

    #[test]
    fn encodes_empty_array() {
        let bytes = encode_votes(&[]);
        // 0x20 (offset to length word) + 0x00 (length zero) = two 32-byte words = 64 bytes.
        assert_eq!(bytes.len(), 64);
        // Last 32 bytes should be all zeros (length = 0).
        assert!(bytes[32..].iter().all(|&b| b == 0));
    }

    #[test]
    fn encodes_single_signature() {
        let bytes = encode_votes(&[sig(0x11)]);
        // Outer offset (32) + length (32) + element offset (32) + element length (32) +
        // padded element data (96 = 65 rounded up to 32-byte multiple) = 224 bytes.
        assert_eq!(bytes.len(), 32 + 32 + 32 + 32 + 96);
    }

    #[test]
    fn encodes_three_signatures() {
        let bytes = encode_votes(&[sig(0x11), sig(0x22), sig(0x33)]);
        // 32 (outer offset) + 32 (length) + 3*32 (per-element offsets) + 3*(32 + 96) bytes =
        // 32 + 32 + 96 + 384 = 544 bytes.
        assert_eq!(bytes.len(), 32 + 32 + 96 + 3 * (32 + 96));
    }

    #[test]
    fn deterministic() {
        let a = encode_votes(&[sig(0xaa), sig(0xbb)]);
        let b = encode_votes(&[sig(0xaa), sig(0xbb)]);
        assert_eq!(a, b);
    }

    #[test]
    fn order_changes_encoding() {
        // Same signers in different order produce different encodings; that's why the pool
        // sorts by signer address before calling this.
        let a = encode_votes(&[sig(0xaa), sig(0xbb)]);
        let b = encode_votes(&[sig(0xbb), sig(0xaa)]);
        assert_ne!(a, b);
    }
}
