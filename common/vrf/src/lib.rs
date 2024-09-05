use anyhow::Result;
use log::{error, info};
use parity_scale_codec::{Decode, Encode};
use schnorrkel::SignatureError;
use serde::{Deserialize, Serialize};
use sp_consensus_babe::VrfOutput;
use sp_core::{
    crypto::{VrfPublic, VrfSecret},
    sr25519::{
        self,
        vrf::{VrfProof, VrfSignData, VrfSignature, VrfTranscript},
    },
};
use thiserror::Error;

use attestor_primitives::AttestorId;
use randomness_primitives::Randomness;

#[derive(Decode, Encode, Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// Represents a proof of inclusion for an attestor.
/// This structure includes the VRF output, VRF proof, and the epoch in which the proof was created.
pub struct ProofOfInclusion {
    /// The output of the VRF (verifiable random function).
    pub output: Vec<u8>,
    /// The proof associated with the VRF output.
    pub proof: Vec<u8>,
    /// The epoch in which the proof was generated.
    pub epoch: u64,
}

/// The context for the VRF
/// This is used to ensure that the VRF output is unique for a specific operation.
const VRF_CONTEXT: &[u8] = b"attestation-vrf";

// For now, we define the maximum u128 value.
const MAX_U128: u128 = u128::MAX;

#[derive(Error, Debug, Clone, PartialEq, Eq)]
/// Error type for VRF-related operations in the attestation mechanism.
pub enum Error {
    /// Error returned when an attestor was not selected due to their random number being above the threshold.
    #[error("attestor was not selected")]
    NotSelected,

    /// Error indicating that the VRF output is invalid.
    #[error("invalid VRF output")]
    InvalidVrfOutput(SignatureError),

    /// Error indicating that the VRF proof is invalid.
    #[error("invalid VRF proof")]
    InvalidVrfProof(SignatureError),

    /// General error indicating that the VRF is invalid.
    #[error("invalid VRF")]
    InvalidVrf,
}

/// Create the proof/signature showing that the attestor
/// was selected to participate in voting.
///
/// In other words, this function creates a proof that the attestor's keys
/// produced a random number that is less than or equal to the `threshold`.
///
/// # Arguments
///
/// * `epoch` - The current epoch for which the proof is being generated.
/// * `attestor_set_size` - The size of the attestor set.
/// * `threshold` - The threshold value for selection.
/// * `randomness` - A source of randomness used in VRF.
/// * `keys` - The SR25519 key pair of the attestor.
/// * `attestor_id` - The ID of the attestor.
/// * `source_block_height` - The block height used for generating the transcript.
///
/// # Returns
///
/// Returns a `ProofOfInclusion` if successful, or an `Error` if the attestor is not selected or there is an issue with the VRF process.
pub fn make_proof_of_inclusion(
    epoch: u64,
    attestor_set_size: u64,
    threshold: u64,
    randomness: &Randomness,
    keys: &sr25519::Pair,
    attestor_id: &AttestorId,
    source_block_height: u64,
) -> Result<ProofOfInclusion, Error> {
    // Create the transcript
    let transcript = make_transcript(randomness, attestor_id, source_block_height);

    // Create the random number
    let random = keys.make_bytes(VRF_CONTEXT, &transcript);

    // Convert the random bytes to a u128
    let random = u128::from_le_bytes(random);
    // Check if the random number is less than the threshold
    let threshold = calculate_threshold(threshold as u128, attestor_set_size as u128);
    if random > threshold {
        info!(
            "attestor was not selected, random: {}, threshold: {}",
            random, threshold
        );
        return Err(Error::NotSelected);
    }

    info!(
        "attestor {:?} was selected, random: {}, threshold: {}",
        attestor_id, random, threshold
    );

    // Create the signing data
    let sign_data = VrfSignData::new(transcript);

    // Sign the data
    let sig = keys.vrf_sign(&sign_data);

    // Return the proof of inclusion
    Ok(ProofOfInclusion {
        output: sig.output.encode(),
        proof: sig.proof.encode(),
        epoch,
    })
}

/// Verifies the provided proof of inclusion to ensure that the attestor was indeed selected based on the VRF output.
///
/// # Arguments
///
/// * `threshold` - The threshold value for attestor selection.
/// * `attestor_set_size` - The size of the attestor set.
/// * `randomness` - A source of randomness used in VRF.
/// * `proof_of_inclusion` - The proof of inclusion to verify.
/// * `attestor_id` - The ID of the attestor.
/// * `source_block_height` - The block height used for generating the transcript.
///
/// # Returns
///
/// Returns `Ok(true)` if the proof is valid and the attestor was selected, otherwise `Ok(false)`.
pub fn verify_proof_of_inclusion(
    threshold: u64,
    attestor_set_size: u64,
    randomness: &Randomness,
    proof_of_inclusion: &ProofOfInclusion,
    attestor_id: &AttestorId,
    source_block_height: u64,
) -> Result<bool, Error> {
    let vrf_input = VrfSignData::new(make_transcript(
        randomness,
        attestor_id,
        source_block_height,
    ));

    let vrf_signature = VrfSignature {
        output: VrfOutput(
            schnorrkel::vrf::VRFOutput::from_bytes(&proof_of_inclusion.output)
                .map_err(Error::InvalidVrfOutput)?,
        ),
        proof: VrfProof(
            schnorrkel::vrf::VRFProof::from_bytes(&proof_of_inclusion.proof)
                .map_err(Error::InvalidVrfProof)?,
        ),
    };

    let attestor_public_key = sr25519::Public(attestor_id.public_key());

    if !attestor_public_key.vrf_verify(&vrf_input, &vrf_signature) {
        error!("failed to verify vrf signature");
        return Ok(false);
    }

    let random_pub = vrf_signature
        .output
        .make_bytes(
            VRF_CONTEXT,
            vrf_input.as_ref(),
            &sr25519::Public(attestor_id.public_key()),
        )
        .map_err(|_| Error::InvalidVrf)?;

    let random_pub = u128::from_le_bytes(random_pub);

    let threshold = calculate_threshold(threshold as u128, attestor_set_size as u128);
    if random_pub > threshold {
        info!(
            "attestor was not selected, random: {}, threshold: {}",
            random_pub, threshold
        );
        return Ok(false);
    }

    info!(
        "attestor {:?} was selected, random: {}, threshold: {}",
        attestor_id, random_pub, threshold
    );

    Ok(true)
}

/// Constructs a VRF transcript, which acts as the input to the VRF.
/// This ensures that the VRF output is tied to specific inputs (randomness, attestor ID, and block height).
///
/// # Arguments
///
/// * `randomness` - The source of randomness used in the VRF.
/// * `attestor_id` - The ID of the attestor.
/// * `source_block_height` - The block height to be included in the transcript.
///
/// # Returns
///
/// Returns a `VrfTranscript` object to be used in the VRF process.
fn make_transcript(
    randomness: &Randomness,
    attestor_id: &AttestorId,
    source_block_height: u64,
) -> VrfTranscript {
    VrfTranscript::new(
        b"attestation_engine",
        &[
            (b"source_block_height", &source_block_height.encode()),
            (b"randomness", randomness.as_ref()),
            (b"id", &attestor_id.encode()),
        ],
    )
}

/// Calculates the selection threshold based on the target sample size and the working set size.
/// This is used to determine the probability of an attestor being selected.
///
/// # Arguments
///
/// * `target_sample_size` - The number of attestors expected to be selected.
/// * `working_size` - The total number of attestors.
///
/// # Returns
///
/// Returns the threshold value as a `u128`.
fn calculate_threshold(target_sample_size: u128, working_size: u128) -> u128 {
    // Calculate the threshold
    (MAX_U128 / working_size) * target_sample_size
}

#[cfg(test)]
mod tests {
    use std::u64;

    use super::*;
    use sp_core::sr25519::Pair;
    use sp_core::{Pair as _, H256};

    #[test]
    fn test_make_proof_of_inclusion() {
        let _ = env_logger::try_init();

        let threshold = 100;
        let attestor_set_size = 100;
        let randomness = Randomness::from(H256::random());
        let keys = Pair::from_string("//Alice", None).unwrap();
        let attestor_id = AttestorId::from_public(keys.public().0);
        let source_block_height = 1;
        let epoch = 1;

        let proof_of_inclusion = make_proof_of_inclusion(
            epoch,
            attestor_set_size,
            threshold,
            &randomness,
            &keys,
            &attestor_id,
            source_block_height,
        )
        .unwrap();

        assert!(verify_proof_of_inclusion(
            threshold,
            attestor_set_size,
            &randomness,
            &proof_of_inclusion,
            &attestor_id,
            source_block_height
        )
        .unwrap());
    }

    #[test]
    fn test_make_proof_of_inclusion_high_attestor_set_fails_probably() {
        let _ = env_logger::try_init();

        let threshold = 1;
        let attestor_set_size = u64::MAX;
        let randomness = Randomness::from(H256::random());
        let keys = Pair::from_string("//Alice", None).unwrap();
        let attestor_id = AttestorId::from_public(keys.public().0);
        let source_block_height = 1;
        let epoch = 1;

        let res = make_proof_of_inclusion(
            epoch,
            attestor_set_size,
            threshold,
            &randomness,
            &keys,
            &attestor_id,
            source_block_height,
        );

        assert_eq!(res, Err(Error::NotSelected));
    }

    #[test]
    fn test_make_proof_of_inclusion_not_selected() {
        let _ = env_logger::try_init();

        let threshold = 100;
        let attestor_set_size = 100;
        let randomness = Randomness::from(H256::random());
        let keys = Pair::from_string("//Alice", None).unwrap();
        let attestor_id = AttestorId::from_public(keys.public().0);
        let source_block_height = 1;
        let epoch = 1;

        let proof_of_inclusion = make_proof_of_inclusion(
            epoch,
            threshold,
            attestor_set_size,
            &randomness,
            &keys,
            &attestor_id,
            source_block_height,
        )
        .unwrap();

        // Increase the source block height to make the attestor not selected
        let source_block_height = 2;

        assert!(!verify_proof_of_inclusion(
            threshold,
            attestor_set_size,
            &randomness,
            &proof_of_inclusion,
            &attestor_id,
            source_block_height
        )
        .unwrap());

        // Or change randomness
        let randomness = Randomness::from(H256::random());

        assert!(!verify_proof_of_inclusion(
            threshold,
            attestor_set_size,
            &randomness,
            &proof_of_inclusion,
            &attestor_id,
            source_block_height
        )
        .unwrap());
    }

    #[test]
    fn test_threshold() {
        let _ = env_logger::try_init();

        let target_sample_size = 100;
        let working_size = 100;

        let threshold = calculate_threshold(target_sample_size, working_size);

        assert_eq!(threshold, 340282366920938463463374607431768211400);
    }
}
