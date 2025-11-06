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
type ConstU1MB = sp_core::ConstU32<1048576>;

// 64 bytes for sr25519 signature
type SR25519SignatureBytes = ConstU32<64>;

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
        _: &mut impl PrecompileHandle,
        message: BoundedBytes<ConstU1MB>,
        signature: BoundedBytes<SR25519SignatureBytes>,
        public_key: H256,
    ) -> EvmResult<bool> {
        let message_bytes: Vec<u8> = message.into();
        let signature_bytes: Vec<u8> = signature.into();

        // Validate signature length
        if signature_bytes.len() != 64 {
            log::debug!(
                "Invalid signature length: expected 64 bytes, got {}",
                signature_bytes.len()
            );
            return Ok(false);
        }

        // Convert signature bytes to sr25519::Signature
        let sig = match sr25519::Signature::try_from(signature_bytes.as_slice()) {
            Ok(s) => s,
            Err(_) => {
                log::debug!("Failed to parse signature");
                return Ok(false);
            }
        };

        // Convert public key to sr25519::Public
        let public = sr25519::Public::from_raw(public_key.0);

        // Verify the signature
        let is_valid = sr25519_verify(&sig, &message_bytes, &public);

        log::debug!(
            "Signature verification result: {}, message_len: {}, public_key: {:?}",
            is_valid,
            message_bytes.len(),
            public_key
        );

        Ok(is_valid)
    }
}
