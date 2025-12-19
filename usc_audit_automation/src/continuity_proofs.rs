use crate::{
    clients::usc::decode::ContinuityProofStatus, SignedAttestation, UniversalSmartContractProvider,
};
use attestor_primitives::{block::Block, Digest};
use subxt::utils::AccountId32;
use tracing::{debug, error};

pub async fn validate_continuity_proof(
    usc_client: &impl UniversalSmartContractProvider,
    att: &SignedAttestation<Digest, AccountId32>,
    genesis_block_number: u64,
    last_finalized_opt: Option<Digest>,
    proof_status: ContinuityProofStatus,
) -> bool {
    debug!("🔍 Validating attestation continuity...");

    let chain_key = att.attestation.chain_key;
    let att_header = att.attestation.header_number;
    let att_prev = att.attestation.prev_digest;

    // ───────────────────────────────────────────────
    // 0. GENESIS CASE
    // ───────────────────────────────────────────────
    if att_header == genesis_block_number {
        debug!("✨ Genesis attestation — special case");

        // Must NOT have a prev_digest
        if att_prev.is_some() {
            debug!("✖ Genesis attestation MUST NOT have prev_digest");
            return false;
        }

        // Must NOT contain any continuity proof blocks
        if !att.continuity_proof.is_empty() {
            debug!("✖ Genesis attestation MUST have empty continuity_proof");
            return false;
        }

        debug!("✅ Genesis attestation valid");
        return true;
    }

    // ───────────────────────────────────────────────
    // 1. NON-GENESIS MUST HAVE prev_digest
    // ───────────────────────────────────────────────
    let prev_digest = match att_prev {
        Some(p) => p,
        None => {
            debug!("✖ Non-genesis attestation missing prev_digest");
            return false;
        }
    };

    if prev_digest.0.iter().all(|b| *b == 0) {
        debug!("✖ prev_digest is all zeros");
        return false;
    }

    // ───────────────────────────────────────────────
    // 2. MUST HAVE last_finalized_opt
    // ───────────────────────────────────────────────
    let last_finalized = match last_finalized_opt {
        Some(v) => v,
        None => {
            debug!("✖ last_finalized_digest missing");
            return false;
        }
    };

    // ───────────────────────────────────────────────
    // 3. HANDLE proof_status BEFORE ANY PROOF LOGIC
    // ───────────────────────────────────────────────

    match proof_status {
        ContinuityProofStatus::Missing => {
            debug!("⚠️ continuity_proof missing in runtime → legacy fallback");

            // Legacy rule (pre-proof): direct link only
            if prev_digest != last_finalized {
                debug!(
                    "✖ Legacy continuity FAILED (prev={:?} != last_finalized={:?})",
                    prev_digest, last_finalized
                );
                return false;
            }

            // Direct link case — check header continuity
            debug!("🔗 Direct link case — checking header continuity only");
            match usc_client
                .get_attestation_header_by_digest(chain_key, prev_digest)
                .await
            {
                Ok(Some(prev_hdr)) => {
                    let expected_header = match prev_hdr.checked_add(1) {
                        Some(h) => h,
                        None => {
                            debug!(
                                "✖ Previous header number is u64::MAX, cannot have a next block"
                            );
                            return false;
                        }
                    };
                    if att_header == expected_header {
                        debug!("✅ Legacy continuity OK (direct link valid)");
                        return true;
                    } else {
                        debug!(
                            "✖ Header mismatch: expected {}, got {}",
                            expected_header, att_header
                        );
                        return false;
                    }
                }
                _ => {
                    debug!("✖ No previous attestation available in storage");
                    return false;
                }
            }
        }

        ContinuityProofStatus::DecodeFailed => {
            debug!("✖ continuity_proof present but failed to decode — MUST fail");
            return false;
        }

        ContinuityProofStatus::Present => {
            // Continue to real continuity proof validation below.
        }
    }

    // ───────────────────────────────────────────────
    // 4. PROOF REQUIRED (proof_status == Present)
    // ───────────────────────────────────────────────
    if att.continuity_proof.is_empty() {
        debug!("✖ continuity_proof empty but required");
        return false;
    }

    let blocks = att.continuity_proof.get_blocks_ref();

    let head = match att.continuity_proof.head() {
        Some(h) => h,
        None => {
            debug!("✖ continuity_proof missing head");
            return false;
        }
    };

    let tail = match att.continuity_proof.tail() {
        Some(t) => t,
        None => {
            debug!("✖ continuity_proof missing tail");
            return false;
        }
    };

    // ───────────────────────────────────────────────
    // 6. HEAD CHECK
    // ───────────────────────────────────────────────
    let head_block: Block = (*head).clone().into();
    let head_digest = head_block.digest;

    if head_digest != prev_digest {
        debug!(
            "✖ continuity_proof head mismatch: expected {:?}, found {:?}",
            prev_digest, head_digest
        );
        return false;
    }

    // ───────────────────────────────────────────────
    // 7. TAIL CHECK — must continue from known finalized
    // ───────────────────────────────────────────────
    let tail_block: Block = tail.clone().into();
    let tail_prev = tail_block.prev_digest;

    match usc_client
        .get_attestation_header_by_digest(chain_key, tail_prev)
        .await
    {
        Ok(Some(prev_hdr)) => {
            let expected_prev_number = match tail_block.block_number.checked_sub(1) {
                Some(n) => n,
                None => {
                    debug!("✖ Tail block number is 0, cannot have a previous block");
                    return false;
                }
            };
            if prev_hdr != expected_prev_number {
                debug!(
                    "✖ Tail mismatch: expected header {}, found {}",
                    expected_prev_number, prev_hdr
                );
                return false;
            }
        }
        Ok(None) => {
            debug!("✖ Tail prev_digest does not exist in storage");
            return false;
        }
        Err(_) => {
            error!("✖ Error while fetching tail prev_digest from storage");
            return false;
        }
    }

    // Starting digest for chain walk
    let mut cursor = tail_prev;

    // ───────────────────────────────────────────────
    // 8. WALK THE PROOF tail → head
    // ───────────────────────────────────────────────
    for serializable in blocks.clone() {
        let block: Block = serializable.into();
        let block_digest = block.digest;
        let block_prev = block.prev_digest;

        debug!(
            "🧩 block_number={}, digest={:?}, prev={:?}, cursor={:?}",
            block.block_number, block_digest, block_prev, cursor
        );

        if cursor != block_prev {
            debug!(
                "✖ Continuity break: expected prev {:?}, found {:?}",
                cursor, block_prev
            );
            return false;
        }

        cursor = block_digest;
    }

    debug!("✅ continuity_proof validated SUCCESSFULLY");
    true
}

// ------------------------------------------------
// Test support (mocks + fragment builders)
// ------------------------------------------------
// ------------------------------------------------
// Test support (continuity fragments + mock client)
// ------------------------------------------------
#[cfg(test)]
mod testsupport {
    use crate::clients::usc::decode::DecodedSignedAttestation;

    use super::*;
    use anyhow::Result;
    use attestor_primitives::{
        attestation_fragment::{AttestationFragment, AttestationFragmentSerializable},
        block::Block,
    };
    use sp_core::H256;

    /// Mock USC client for continuity-proof validation tests.
    ///
    /// Rule:
    /// - interpret last 8 bytes of digest as a mock "header_number"
    /// - allow headers 0..=20, return None for anything else
    pub struct MockUscClientValid;

    impl UniversalSmartContractProvider for MockUscClientValid {
        async fn get_attestation_header_by_digest(
            &self,
            _chain_key: u64,
            digest: Digest,
        ) -> Result<Option<u64>> {
            let n = u64::from_be_bytes(digest.0[24..32].try_into().unwrap());

            if n <= 20 {
                Ok(Some(n))
            } else {
                Ok(None)
            }
        }

        async fn fetch_last_digest(&self, _: u64) -> Result<Option<Digest>> {
            Ok(None)
        }

        async fn get_attestation_by_digest(
            &self,
            _: u64,
            _: Digest,
        ) -> Result<Option<DecodedSignedAttestation>> {
            Ok(None)
        }

        async fn get_last_attestation_checkpoint(
            &self,
            _: u64,
        ) -> Result<Option<attestor_primitives::AttestationCheckpoint>> {
            Ok(None)
        }

        async fn get_checkpoint_interval(&self, _: u64) -> Result<Option<u32>> {
            Ok(None)
        }

        async fn get_attestation_interval(&self, _: u64) -> Result<Option<u64>> {
            Ok(None)
        }

        async fn get_attestation_vote_acceptance_window(&self, _: u64) -> Result<Option<u64>> {
            Ok(None)
        }
    }

    // ------------------------------------------------
    // Construct a valid continuity fragment (1 → 2 → 3)
    // ------------------------------------------------
    pub fn make_valid_fragment() -> AttestationFragmentSerializable {
        let mut fragment = AttestationFragment::new(3);

        let mut prev = H256::from_low_u64_be(0);
        for i in 1..=3 {
            let mut block = Block::new(i, H256::default());
            block.prev_digest = prev;
            block.digest = H256::from_low_u64_be(i);

            fragment.try_append_block(block.clone()).unwrap();
            prev = block.digest;
        }

        AttestationFragmentSerializable::from(&fragment)
    }

    // ------------------------------------------------
    // Create a tail-breaking fragment
    // ------------------------------------------------
    pub fn make_tail_break_fragment() -> AttestationFragmentSerializable {
        let mut fragment = AttestationFragment::new(3);

        let mut prev = H256::from_low_u64_be(0);
        for i in 1..=3 {
            let mut block = Block::new(i, H256::default());

            block.prev_digest = if i == 1 {
                H256::from_low_u64_be(9999) // ❌ broken tail digest
            } else {
                prev
            };

            block.digest = H256::from_low_u64_be(i);
            fragment.try_append_block(block.clone()).unwrap();
            prev = block.digest;
        }

        AttestationFragmentSerializable::from(&fragment)
    }
}

// ------------------------------------------------
// Tests
// ------------------------------------------------
#[cfg(test)]
mod tests {
    use super::testsupport::{make_tail_break_fragment, make_valid_fragment, MockUscClientValid};
    use super::*;
    use crate::clients::usc::decode::ContinuityProofStatus;
    use attestor_primitives::{
        attestation_fragment::AttestationFragmentSerializable, AttestationData,
    };
    use sp_core::H256;

    // --- 1. VALID CONTINUITY PROOF ----------------------------------------
    #[tokio::test]
    async fn test_validate_continuity_proof_valid() {
        let client = MockUscClientValid;
        let continuity_proof = make_valid_fragment();

        // continuity_proof.head().digest must match att.prev_digest
        let head_digest = continuity_proof.head().unwrap().digest;

        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 4,
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                prev_digest: Some(H256::from_slice(&head_digest.0)),
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof,
        };

        let last_finalized = Some(H256::from_low_u64_be(0));

        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            last_finalized,
            ContinuityProofStatus::Present,
        )
        .await;

        assert!(result, "Expected valid continuity proof");
    }

    // --- 2. INVALID HEAD DIGEST -------------------------------------------
    #[tokio::test]
    async fn test_validate_continuity_proof_invalid_head() {
        let client = MockUscClientValid;
        let continuity_proof = make_valid_fragment();

        // prev_digest intentionally mismatches fragment head
        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 4,
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                prev_digest: Some(H256::from_low_u64_be(999)), // ❌ mismatch
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof,
        };

        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            Some(H256::from_low_u64_be(0)),
            ContinuityProofStatus::Present,
        )
        .await;

        assert!(!result, "Head mismatch should invalidate continuity proof");
    }

    // --- 3. MISSING CONTINUITY PROOF (NON-GENESIS) ------------------------
    #[tokio::test]
    async fn test_validate_continuity_proof_missing() {
        let client = MockUscClientValid;

        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 5,
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                prev_digest: Some(H256::from_low_u64_be(3)),
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof: AttestationFragmentSerializable { blocks: vec![] }, // ❌ empty
        };

        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            Some(H256::from_low_u64_be(0)),
            ContinuityProofStatus::Present,
        )
        .await;

        assert!(
            !result,
            "Missing continuity proof should fail for non-genesis attestations"
        );
    }

    // --- 4. GENESIS CASE --------------------------------------------------
    #[tokio::test]
    async fn test_validate_continuity_proof_genesis() {
        let client = MockUscClientValid;

        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 100, // genesis
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                prev_digest: None, // ❗ correct for genesis
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof: AttestationFragmentSerializable { blocks: vec![] },
        };

        // For a genesis attestation, proof_status can still be Present
        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            None,
            ContinuityProofStatus::Present,
        )
        .await;

        assert!(result, "Genesis attestation must be accepted without proof");
    }

    // --- 5. BROKEN TAIL LINK ----------------------------------------------
    #[tokio::test]
    async fn test_validate_continuity_proof_tail_break() {
        let client = MockUscClientValid;
        let continuity_proof = make_tail_break_fragment();

        let head_digest = continuity_proof.head().unwrap().digest;

        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 4,
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                prev_digest: Some(H256::from_slice(&head_digest.0)),
            },
            signature: [0; 96],
            attestors: vec![],
            continuity_proof,
        };

        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            Some(H256::from_low_u64_be(0)),
            ContinuityProofStatus::Present,
        )
        .await;

        assert!(
            !result,
            "Tail-break continuity fragment should fail validation"
        );
    }

    #[tokio::test]
    async fn test_validate_continuity_proof_missing_field_legacy_runtime() {
        let client = MockUscClientValid;

        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 11,
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                // Pretend the previous digest matches the last finalized digest.
                prev_digest: Some(H256::from_low_u64_be(10)),
            },
            signature: [0; 96],
            attestors: vec![],
            // continuity_proof exists structurally but is logically "absent"
            // because proof_status = Missing
            continuity_proof: AttestationFragmentSerializable { blocks: vec![] },
        };

        let last_finalized = Some(H256::from_low_u64_be(10));

        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            last_finalized,
            ContinuityProofStatus::Missing,
        )
        .await;

        assert!(
            result,
            "Expected legacy runtime (Missing) to accept direct-link attestation"
        );
    }

    #[tokio::test]
    async fn test_validate_continuity_proof_decode_failed() {
        let client = MockUscClientValid;

        let attestation = SignedAttestation {
            attestation: AttestationData {
                chain_key: 2,
                header_number: 20,
                header_hash: H256::from_slice(&[1; 32]),
                root: H256::from_slice(&[0; 32]),
                prev_digest: Some(H256::from_low_u64_be(19)),
            },
            signature: [0; 96],
            attestors: vec![],
            // continuity_proof supplied but malformed → proof_status::DecodeFailed
            continuity_proof: AttestationFragmentSerializable { blocks: vec![] },
        };

        let last_finalized = Some(H256::from_low_u64_be(19));

        let result = validate_continuity_proof(
            &client,
            &attestation,
            100,
            last_finalized,
            ContinuityProofStatus::DecodeFailed,
        )
        .await;

        assert!(
            !result,
            "Expected continuity_proof DecodeFailed to hard-fail validation"
        );
    }
}
