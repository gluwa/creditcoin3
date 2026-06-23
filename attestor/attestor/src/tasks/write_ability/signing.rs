//! Message-vote signing (confluence §7.3 A5 / §6.3).
//!
//! Message votes use **ECDSA / secp256k1** to match the reference `EOAValidator`, distinct from the
//! BLS scheme used for block attestations (§6.7). Each attestor signs the raw 32-byte `messageHash`
//! directly — **no** EIP-191 / `personal_sign` prefix — producing a 65-byte `(r, s, v)` signature
//! that `ecrecover` on-chain maps back to the signer's EVM address.
//!
//! The EVM signing key is derived deterministically from the attestor's existing secret with domain
//! separation, so an operator manages one secret and gets a stable EVM address to register in the
//! validator's attestor set (§6.3 option B). The signed bytes and recovery here are byte-identical
//! to what `message-relayer` recovers (`recover_address_from_prehash` over the same 65 bytes).

use alloy::primitives::{keccak256, Address, PrimitiveSignature, B256};
use alloy::signers::local::PrivateKeySigner;
use alloy::signers::SignerSync;
use anyhow::{Context, Result};

/// Domain-separation tag so the derived EVM key is independent of the BLS / ed25519 / Substrate
/// keys derived from the same seed.
const EVM_SIGNER_DOMAIN: &[u8] = b"usc/write-ability/evm-signer/v1";

/// Holds the attestor's EVM message-vote signing key.
pub struct MessageSigner {
    signer: PrivateKeySigner,
    address: Address,
}

impl MessageSigner {
    /// Derive the EVM signing key from the attestor's 32-byte seed (domain-separated).
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self> {
        let mut preimage = Vec::with_capacity(EVM_SIGNER_DOMAIN.len() + seed.len());
        preimage.extend_from_slice(EVM_SIGNER_DOMAIN);
        preimage.extend_from_slice(seed);
        let derived = keccak256(&preimage);
        let signer = PrivateKeySigner::from_slice(derived.as_slice())
            .context("derived EVM signing key is invalid (zero or >= curve order)")?;
        let address = signer.address();
        Ok(Self { signer, address })
    }

    /// The EVM address that must be registered in the on-chain `EOAValidator` attestor set.
    #[must_use]
    pub fn address(&self) -> Address {
        self.address
    }

    /// Sign the raw `messageHash` (no EIP-191 prefix) → 65-byte `(r, s, v)`.
    pub fn sign(&self, message_hash: &B256) -> Result<[u8; 65]> {
        let sig = self
            .signer
            .sign_hash_sync(message_hash)
            .context("ECDSA sign over messageHash failed")?;
        Ok(sig.as_bytes())
    }
}

/// Recover the EVM signer address from a 65-byte signature over `message_hash`. Mirrors the
/// relayer's `recover_signer` so both sides agree on who signed.
pub fn recover_signer(message_hash: &B256, raw: &[u8; 65]) -> Result<Address> {
    let sig: PrimitiveSignature = raw[..]
        .try_into()
        .map_err(|e| anyhow::anyhow!("malformed signature bytes: {e}"))?;
    sig.recover_address_from_prehash(message_hash)
        .map_err(|e| anyhow::anyhow!("ecrecover failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_key_is_deterministic() {
        let seed = [7u8; 32];
        let a = MessageSigner::from_seed(&seed).unwrap();
        let b = MessageSigner::from_seed(&seed).unwrap();
        assert_eq!(a.address(), b.address());
    }

    #[test]
    fn different_seeds_give_different_addresses() {
        let a = MessageSigner::from_seed(&[1u8; 32]).unwrap();
        let b = MessageSigner::from_seed(&[2u8; 32]).unwrap();
        assert_ne!(a.address(), b.address());
    }

    #[test]
    fn sign_then_recover_round_trips() {
        let signer = MessageSigner::from_seed(&[42u8; 32]).unwrap();
        let hash = B256::from([0x11u8; 32]);
        let sig = signer.sign(&hash).unwrap();
        let recovered = recover_signer(&hash, &sig).unwrap();
        assert_eq!(recovered, signer.address());
    }

    #[test]
    fn recovery_is_hash_sensitive() {
        let signer = MessageSigner::from_seed(&[42u8; 32]).unwrap();
        let sig = signer.sign(&B256::from([0x11u8; 32])).unwrap();
        // A signature over a different hash must not recover to our address.
        let other = recover_signer(&B256::from([0x22u8; 32]), &sig).unwrap();
        assert_ne!(other, signer.address());
    }
}
