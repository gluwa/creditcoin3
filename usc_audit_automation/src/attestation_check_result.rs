use crate::{
    calculate_usc_and_source_chain_block_diff, CheckPointCreatedWithinRangeChecker,
    MAX_ALLOWED_BLOCK_HEIGHT_DIFF,
};
use attestor_primitives::SignedAttestation;
use ethers::types::U64;
use sp_core::H256;
use subxt::utils::AccountId32;

#[derive(Debug)]
pub struct AttestationInfo {
    pub attestor_best_block_number: u64,
    pub attestation_merkle_root: String,
}

#[derive(Debug)]
pub struct AttestationCheckResult {
    pub attestation_info: AttestationInfo,
    pub block_height_diff: i128,
    pub ethereum_block_info: EthereumBlockInfo,
    pub check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker,
}

impl AttestationCheckResult {
    pub fn is_block_height_exceeded(&self) -> bool {
        self.block_height_diff > MAX_ALLOWED_BLOCK_HEIGHT_DIFF
    }
    pub fn header_hash_matches(&self) -> bool {
        Some(self.attestation_info.attestor_best_block_number)
            == self
                .ethereum_block_info
                .fetched_ethereum_block_number_by_hash
    }
    pub fn is_checkpoint_in_range(&self) -> bool {
        self.check_point_created_in_range_checker
            .checkpoint_created_within_range
    }
    pub fn merkle_roots_match(&self) -> bool {
        self.attestation_info.attestation_merkle_root
            == self
                .ethereum_block_info
                .calculated_ethereum_block_merkle_root
    }
}

#[derive(Debug)]
pub struct EthereumBlockInfo {
    pub latest_ethereum_block_number: u64,
    pub calculated_ethereum_block_merkle_root: String,
    pub fetched_ethereum_block_number_by_hash: Option<u64>,
}

pub fn compute_attestation_check_result(
    latest_signed_attestation: &SignedAttestation<H256, AccountId32>,
    latest_ethereum_block_number: u64,
    calculated_ethereum_block_merkle_root: &str,
    fetched_ethereum_block_number_by_hash: Option<U64>,
    check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker,
) -> AttestationCheckResult {
    let attestor_best_block_number = latest_signed_attestation.attestation.header_number;
    let block_height_diff = calculate_usc_and_source_chain_block_diff(
        attestor_best_block_number,
        latest_ethereum_block_number,
    );

    let fetched_ethereum_block_number_by_hash: Option<u64> =
        fetched_ethereum_block_number_by_hash.map(|block| block.as_u64());

    let attestation_merkle_root = format!(
        "0x{}",
        hex::encode(latest_signed_attestation.attestation.root)
    );

    let attestation_info = AttestationInfo {
        attestor_best_block_number,
        attestation_merkle_root,
    };
    let ethereum_block_info = EthereumBlockInfo {
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root: calculated_ethereum_block_merkle_root.to_string(),
        fetched_ethereum_block_number_by_hash,
    };

    AttestationCheckResult {
        attestation_info,
        block_height_diff,
        ethereum_block_info,
        check_point_created_in_range_checker,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use attestor_primitives::{Attestation, SignedAttestation};
    use sp_core::H256;
    use subxt::utils::AccountId32;

    fn dummy_attestation(
        header_number: u64,
        root: [u8; 32],
    ) -> SignedAttestation<H256, AccountId32> {
        SignedAttestation {
            attestation: Attestation {
                chain_key: 1,
                header_number,
                header_hash: H256::zero(),
                root,
                prev_digest: None,
            },
            signature: [0u8; 96],
            attestors: vec![],
        }
    }

    #[test]
    fn test_attestation_check_result_is_block_height_exceeded_when_block_height_diff_exceeds_threshold(
    ) {
        let block_height_diff: i128 = 100;

        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
            },
            block_height_diff,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(result.is_block_height_exceeded());
    }

    #[test]
    fn test_attestation_check_result_is_not_block_height_exceeded_when_block_height_diff_within_threshold(
    ) {
        let block_height_diff: i128 = 10;

        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
            },
            block_height_diff,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(!result.is_block_height_exceeded());
    }

    #[test]
    fn test_attatation_check_result_header_hash_matches() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 100,
                attestation_merkle_root: String::new(),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: Some(100),
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(result.header_hash_matches());
    }

    #[test]
    fn test_attatation_check_result_header_hash_does_not_match() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 100,
                attestation_merkle_root: String::new(),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: Some(99),
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(!result.header_hash_matches());
    }

    #[test]
    fn test_attatation_check_result_is_checkpoint_in_range() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(result.is_checkpoint_in_range());
    }

    #[test]
    fn test_attatation_check_result_is_not_checkpoint_in_range() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: false,
            },
        };

        assert!(!result.is_checkpoint_in_range());
    }

    #[test]
    fn test_attatation_check_result_merkle_roots_match() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: "0xdeadbeef".to_string(),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: "0xdeadbeef".to_string(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(result.merkle_roots_match());
    }

    #[test]
    fn test_attatation_check_result_merkle_roots_do_not_match() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: "0xdeadbeef".to_string(),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: "0xfeedface".to_string(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckPointCreatedWithinRangeChecker {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
        };

        assert!(!result.merkle_roots_match());
    }

    #[test]
    fn test_all_checks_pass() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = format!("0x{}", hex::encode([1u8; 32]));
        let checkpoint_created_in_range_checker = CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };
        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
        );

        assert!(!result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(result.merkle_roots_match());
    }

    #[test]
    fn test_block_height_exceeded_only() {
        let attestation = dummy_attestation(10, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(10u64.into());
        let calculated_ethereum_block_merkle_root = format!("0x{}", hex::encode([1u8; 32]));
        let checkpoint_created_in_range_checker = CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number: 5,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };

        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
        );

        assert!(result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(result.merkle_roots_match());
    }

    #[test]
    fn test_header_hash_mismatch_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(99u64.into());
        let calculated_ethereum_block_merkle_root = format!("0x{}", hex::encode([1u8; 32]));
        let checkpoint_created_in_range_checker = CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };

        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
        );

        assert!(!result.is_block_height_exceeded());
        assert!(!result.header_hash_matches());
        assert!(result.merkle_roots_match());
    }

    #[test]
    fn test_merkle_root_mismatch_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = "0xdeadbeef".to_string();
        let checkpoint_created_in_range_checker = CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };

        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
        );

        assert!(!result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(!result.merkle_roots_match());
    }
}
