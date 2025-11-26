#![cfg_attr(not(feature = "std"), no_std)]

use core::marker::PhantomData;
use fp_evm::PrecompileHandle;
use precompile_utils::prelude::*;
use sp_core::{sr25519, ConstU32, H256};
use sp_io::crypto::sr25519_verify;
use sp_std::vec::Vec;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Precompile for verifying Substrate sr25519 signatures.
pub struct SignatureVerifierPrecompile<Runtime>(PhantomData<Runtime>);

// 1MB limit for message size
type ConstU1MB = ConstU32<1048576>;

// 64 bytes for sr25519 signature
type SR25519SignatureBytes = ConstU32<64>;

// Gas costs for signature verification
// Base cost comparable to ECRECOVER (3000 gas) which does ECDSA verification + recovery
// SR25519 verification is similar in computational cost to ECDSA verification but its more compute intensive, so 3500 gas
const GAS_BASE_VERIFY: u64 = 3_500; // Base cost for cryptographic signature verification
const GAS_PER_MESSAGE_BYTE: u64 = 3; // Per-byte cost for message processing

#[precompile_utils::precompile]
impl<Runtime> SignatureVerifierPrecompile<Runtime>
where
    Runtime: pallet_evm::Config,
{
    /// Verifies an sr25519 signature.
    ///
    /// # Arguments
    /// * `message` - The message that was signed
    /// * `signature` - The 64-byte sr25519 signature
    /// * `public_key` - The 32-byte sr25519 public key
    ///
    /// # Returns
    /// * `bool` - true if the signature is valid, false otherwise
    #[precompile::public("verify(bytes,bytes,bytes32)")]
    fn verify(
        handle: &mut impl PrecompileHandle,
        message: BoundedBytes<ConstU1MB>,
        signature: BoundedBytes<SR25519SignatureBytes>,
        public_key: H256,
    ) -> EvmResult<bool> {
        // Charge base cost for the signature verification operation
        handle.record_cost(GAS_BASE_VERIFY)?;

        let message_bytes: Vec<u8> = message.into();
        let signature_bytes: Vec<u8> = signature.into();

        // Charge per-byte cost for processing the message
        handle.record_cost(GAS_PER_MESSAGE_BYTE.saturating_mul(message_bytes.len() as u64))?;

        if signature_bytes.len() != 64 {
            return Err(revert("Invalid signature length: must be exactly 64 bytes"));
        }

        let mut sig_raw = [0u8; 64];
        sig_raw.copy_from_slice(&signature_bytes);
        let sig = sr25519::Signature::from_raw(sig_raw);

        // Convert public key to sr25519::Public
        let public = sr25519::Public::from_raw(public_key.0);

        // Verify the signature
        let is_valid = sr25519_verify(&sig, &message_bytes, &public);

        Ok(is_valid)
    }
}
