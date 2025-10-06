use crate::calculate_usc_and_source_chain_block_diff;
use attestor_primitives::SignedAttestation;
use ethers::types::U64;
use sp_core::H256;
use subxt::utils::AccountId32;

const MAX_ALLOWED_BLOCK_HEIGHT_DIFF: i128 = 50;

fn block_height_exceeded(diff: i128) -> bool {
    diff > MAX_ALLOWED_BLOCK_HEIGHT_DIFF
}

fn header_hash_matches(block_by_hash: Option<u64>, attestor_block: u64) -> bool {
    block_by_hash == Some(attestor_block)
}

fn merkle_roots_match(calculated: &str, attested: &str) -> bool {
    calculated == attested
}

#[derive(Debug)]
pub struct AttestationCheckResult {
    pub block_height_diff: i128,
    pub block_height_exceeded: bool,
    pub header_hash_matches: bool,
    pub merkle_roots_match: bool,
    pub fetched_ethereum_block_number_by_hash: Option<u64>,
    pub attestor_best_block_number: u64,
    pub latest_ethereum_block_number: u64,
    pub calculated_ethereum_block_merkle_root: String,
    pub attestation_merkle_root: String,
}

pub fn compute_attestation_check_result(
    latest_signed_attestation: &SignedAttestation<H256, AccountId32>,
    latest_ethereum_block_number: u64,
    calculated_ethereum_block_merkle_root: &str,
    fetched_ethereum_block_number_by_hash: Option<U64>,
) -> AttestationCheckResult {
    let attestor_best_block_number = latest_signed_attestation.attestation.header_number;
    let block_height_diff = calculate_usc_and_source_chain_block_diff(
        attestor_best_block_number,
        latest_ethereum_block_number,
    );
    let block_height_exceeded = block_height_exceeded(block_height_diff);

    let fetched_ethereum_block_number_by_hash: Option<u64> =
        fetched_ethereum_block_number_by_hash.map(|block| block.as_u64());
    let header_hash_matches = header_hash_matches(
        fetched_ethereum_block_number_by_hash,
        attestor_best_block_number,
    );

    let attestation_merkle_root = format!(
        "0x{}",
        hex::encode(latest_signed_attestation.attestation.root)
    );
    let merkle_roots_match = merkle_roots_match(
        calculated_ethereum_block_merkle_root,
        &attestation_merkle_root,
    );

    AttestationCheckResult {
        block_height_diff,
        block_height_exceeded,
        header_hash_matches,
        merkle_roots_match,
        fetched_ethereum_block_number_by_hash,
        attestor_best_block_number,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root: calculated_ethereum_block_merkle_root.to_string(),
        attestation_merkle_root,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use attestor_primitives::{Attestation, SignedAttestation};
    use sp_core::H256;
    use subxt::utils::AccountId32;

    #[test]
    fn test_block_height_exceeded() {
        assert!(!block_height_exceeded(10)); // below threshold
        assert!(!block_height_exceeded(50)); // equal to threshold
        assert!(block_height_exceeded(51)); // above threshold
        assert!(block_height_exceeded(100)); // well above threshold
    }

    #[test]
    fn test_header_hash_matches() {
        assert!(header_hash_matches(Some(42), 42)); // matches
        assert!(!header_hash_matches(Some(43), 42)); // does not match
        assert!(!header_hash_matches(None, 42)); // None does not match
    }

    #[test]
    fn test_merkle_roots_match() {
        assert!(merkle_roots_match("abc", "abc")); // matches
        assert!(!merkle_roots_match("abc", "def")); // does not match
        assert!(!merkle_roots_match("", "abc")); // empty does not match
        assert!(merkle_roots_match("", "")); // both empty matches
    }

    fn dummy_attestation(
        header_number: u64,
        root: [u8; 32],
    ) -> SignedAttestation<H256, AccountId32> {
        SignedAttestation {
            attestation: Attestation {
                chain_key: 2,
                header_number,
                header_hash: H256::from_slice(&[0u8; 32]),
                root,
                prev_digest: None,
            },
            signature: [0u8; 96],
            attestors: vec![],
        }
    }

    #[test]
    fn test_all_checks_pass() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let eth_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_merkle_root = format!("0x{}", hex::encode([1u8; 32]));

        let result = compute_attestation_check_result(
            &attestation,
            eth_block_number,
            &calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
        );

        assert!(!result.block_height_exceeded);
        assert!(result.header_hash_matches);
        assert!(result.merkle_roots_match);
    }

    #[test]
    fn test_block_height_exceeded_only() {
        let attestation = dummy_attestation(10, [1u8; 32]);
        let eth_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(10u64.into());
        let calculated_merkle_root = format!("0x{}", hex::encode([1u8; 32]));

        let result = compute_attestation_check_result(
            &attestation,
            eth_block_number,
            &calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
        );

        assert!(result.block_height_exceeded);
        assert!(result.header_hash_matches);
        assert!(result.merkle_roots_match);
    }

    #[test]
    fn test_header_hash_mismatch_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let eth_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(99u64.into());
        let calculated_merkle_root = format!("0x{}", hex::encode([1u8; 32]));

        let result = compute_attestation_check_result(
            &attestation,
            eth_block_number,
            &calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
        );

        assert!(!result.block_height_exceeded);
        assert!(!result.header_hash_matches);
        assert!(result.merkle_roots_match);
    }

    #[test]
    fn test_merkle_root_mismatch_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let eth_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_merkle_root = "0xdeadbeef".to_string();

        let result = compute_attestation_check_result(
            &attestation,
            eth_block_number,
            &calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
        );

        assert!(!result.block_height_exceeded);
        assert!(result.header_hash_matches);
        assert!(!result.merkle_roots_match);
    }
}
