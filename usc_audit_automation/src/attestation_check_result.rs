use crate::{
    calculate_usc_and_source_chain_block_diff, CheckpointCreatedWithinRangeResult,
    GraphQLAttestationCheckResult, MAX_ALLOWED_BLOCK_HEIGHT_DIFF,
};
use attestor_primitives::SignedAttestation;
use ethers::types::U64;
use sp_core::H256;
use subxt::utils::AccountId32;

#[derive(Debug)]
pub struct AttestationInfo {
    pub attestor_best_block_number: u64,
    pub attestation_merkle_root: String,
    pub signed_attestation: SignedAttestation<H256, AccountId32>,
}

#[derive(Debug)]
pub struct AttestationCheckResult {
    pub attestation_info: AttestationInfo,
    pub block_height_diff: i128,
    pub ethereum_block_info: EthereumBlockInfo,
    pub check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult,
    pub maybe_elected_attestors: Option<Vec<AccountId32>>,
    pub graphql_attestation_check_result: Option<GraphQLAttestationCheckResult>,
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
    pub fn get_unelected_attestors(&self) -> Vec<AccountId32> {
        if let Some(elected_attestors) = &self.maybe_elected_attestors {
            let unelected_attestors = self
                .attestation_info
                .signed_attestation
                .attestors
                .iter()
                .filter(|attestor| !elected_attestors.contains(attestor))
                .cloned()
                .collect::<Vec<_>>();

            return unelected_attestors;
        }

        Vec::new()
    }
    pub fn last_checkpoint_header_matches_checkpoint_header_in_graphql(&self) -> bool {
        self.graphql_attestation_check_result
            .as_ref()
            .map(|res| {
                res.checkpoint_chain_node
                    .checkpoint_number
                    .parse::<u64>()
                    .unwrap_or_default()
                    == self
                        .check_point_created_in_range_checker
                        .last_checkpoint_block_number
            })
            .unwrap_or_default()
    }
    pub fn last_checkpoint_digest_matches_checkpoint_digest_in_graphql(&self) -> bool {
        self.graphql_attestation_check_result
            .as_ref()
            .map(|res| {
                res.checkpoint_chain_node.last_attested_digest
                    == hex::encode(
                        self.attestation_info
                            .signed_attestation
                            .attestation
                            .digest()
                            .as_bytes(),
                    )
            })
            .unwrap_or_default()
    }
    pub fn last_attestation_header_number_matches_attestation_header_number_in_graphql(
        &self,
    ) -> bool {
        self.graphql_attestation_check_result
            .as_ref()
            .map(|res| {
                res.attestation_node
                    .header_number
                    .parse::<u64>()
                    .unwrap_or_default()
                    == self.attestation_info.signed_attestation.header_number()
            })
            .unwrap_or_default()
    }
    pub fn last_attestation_digest_matches_attestation_digest_in_graphql(&self) -> bool {
        self.graphql_attestation_check_result
            .as_ref()
            .map(|res| {
                res.attestation_node.digest
                    == hex::encode(
                        self.attestation_info
                            .signed_attestation
                            .attestation
                            .digest()
                            .as_bytes(),
                    )
            })
            .unwrap_or_default()
    }
    pub fn last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql(&self) -> bool {
        self.graphql_attestation_check_result
            .as_ref()
            .map(|res| {
                res.attestation_node.prev_digest
                    == self
                        .attestation_info
                        .signed_attestation
                        .attestation
                        .prev_digest()
                        .map(|digest| hex::encode(digest.as_bytes()))
                        .unwrap_or_default()
            })
            .unwrap_or_default()
    }
    pub fn last_attesation_root_matches_attestation_root_in_graphql(&self) -> bool {
        self.graphql_attestation_check_result
            .as_ref()
            .map(|res| {
                res.attestation_node.root
                    == hex::encode(self.attestation_info.signed_attestation.attestation.root)
            })
            .unwrap_or_default()
    }
}

#[derive(Debug)]
pub struct EthereumBlockInfo {
    pub latest_ethereum_block_number: u64,
    pub calculated_ethereum_block_merkle_root: String,
    pub fetched_ethereum_block_number_by_hash: Option<u64>,
}
#[derive(Debug)]
pub struct BlockHeightDiffChecker {
    pub block_height_diff: i128,
    pub block_height_exceeded: bool,
}

pub fn compute_attestation_check_result(
    latest_signed_attestation: &SignedAttestation<H256, AccountId32>,
    latest_ethereum_block_number: u64,
    calculated_ethereum_block_merkle_root: &str,
    fetched_ethereum_block_number_by_hash: Option<U64>,
    check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult,
    maybe_elected_attestors: Option<Vec<AccountId32>>,
    graphql_attestation_check_result: Option<GraphQLAttestationCheckResult>,
) -> AttestationCheckResult {
    let attestor_best_block_number = latest_signed_attestation.attestation.header_number;
    let block_height_diff = calculate_usc_and_source_chain_block_diff(
        attestor_best_block_number,
        latest_ethereum_block_number,
    );

    let fetched_ethereum_block_number_by_hash: Option<u64> =
        fetched_ethereum_block_number_by_hash.map(|block| block.as_u64());

    let attestation_merkle_root = hex::encode(latest_signed_attestation.attestation.root);

    let attestation_info = AttestationInfo {
        attestor_best_block_number,
        attestation_merkle_root,
        signed_attestation: latest_signed_attestation.clone(),
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
        maybe_elected_attestors,
        graphql_attestation_check_result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AttestationCheckpointChainNode, AttestationNode, GraphQLAttestationCheckResult};
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
                prev_digest: Some(H256::zero()),
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
                signed_attestation: dummy_attestation(0, [0u8; 32]),
            },
            block_height_diff,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
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
                signed_attestation: dummy_attestation(0, [0u8; 32]),
            },
            block_height_diff,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(!result.is_block_height_exceeded());
    }

    #[test]
    fn test_attatation_check_result_header_hash_matches() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 100,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(100, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: Some(100),
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(result.header_hash_matches());
    }

    #[test]
    fn test_attatation_check_result_header_hash_does_not_match() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 100,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(100, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: Some(99),
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(!result.header_hash_matches());
    }

    #[test]
    fn test_attatation_check_result_is_checkpoint_in_range() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(0, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(result.is_checkpoint_in_range());
    }

    #[test]
    fn test_attatation_check_result_is_not_checkpoint_in_range() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(0, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: false,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(!result.is_checkpoint_in_range());
    }

    #[test]
    fn test_attatation_check_result_merkle_roots_match() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: "0xdeadbeef".to_string(),
                signed_attestation: dummy_attestation(0, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: "0xdeadbeef".to_string(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(result.merkle_roots_match());
    }

    #[test]
    fn test_attatation_check_result_merkle_roots_do_not_match() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: "0xdeadbeef".to_string(),
                signed_attestation: dummy_attestation(0, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: "0xfeedface".to_string(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 0,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: None,
        };

        assert!(!result.merkle_roots_match());
    }

    #[test]
    fn test_all_checks_pass() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
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
            None,
            Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "50".to_string(),
                    last_attested_digest: hex::encode(attestation.attestation.digest().as_bytes()),
                },
                attestation_node: AttestationNode {
                    header_number: "100".to_string(),
                    digest: hex::encode(attestation.attestation.digest().as_bytes()),
                    prev_digest: "None".to_string(),
                    root: hex::encode(attestation.attestation.root),
                },
            }),
        );

        assert!(!result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(result.merkle_roots_match());
        assert!(result.is_checkpoint_in_range());
        assert!(result.get_unelected_attestors().is_empty());
        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
    }

    #[test]
    fn test_block_height_exceeded_only() {
        let attestation = dummy_attestation(10, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(10u64.into());
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
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
            None,
            Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "5".to_string(),
                    last_attested_digest: hex::encode(attestation.attestation.digest().as_bytes()),
                },
                attestation_node: AttestationNode {
                    header_number: "10".to_string(),
                    digest: hex::encode(attestation.attestation.digest().as_bytes()),
                    prev_digest: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    root: hex::encode(attestation.attestation.root),
                },
            }),
        );

        assert!(result.header_hash_matches());
        assert!(result.merkle_roots_match());
        assert!(result.is_checkpoint_in_range());
        assert!(result.get_unelected_attestors().is_empty());
        assert!(result.is_block_height_exceeded());
        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
        assert!(result.last_attestation_digest_matches_attestation_digest_in_graphql());
        assert!(result.last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql());
        assert!(result.last_attesation_root_matches_attestation_root_in_graphql());
    }

    #[test]
    fn test_header_hash_mismatch_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(99u64.into());
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
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
            None,
            Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "50".to_string(),
                    last_attested_digest: hex::encode(attestation.attestation.digest().as_bytes()),
                },
                attestation_node: AttestationNode {
                    header_number: "100".to_string(),
                    digest: hex::encode(attestation.attestation.digest().as_bytes()),
                    prev_digest: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    root: hex::encode(attestation.attestation.root),
                },
            }),
        );

        assert!(!result.is_block_height_exceeded());
        assert!(result.merkle_roots_match());
        assert!(result.is_checkpoint_in_range());
        assert!(result.get_unelected_attestors().is_empty());
        assert!(!result.header_hash_matches());
        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
        assert!(result.last_attestation_digest_matches_attestation_digest_in_graphql());
        assert!(result.last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql());
        assert!(result.last_attesation_root_matches_attestation_root_in_graphql());
    }

    #[test]
    fn test_merkle_root_mismatch_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = "0xdeadbeef".to_string();
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
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
            None,
            Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "50".to_string(),
                    last_attested_digest: hex::encode(attestation.attestation.digest().as_bytes()),
                },
                attestation_node: AttestationNode {
                    header_number: "100".to_string(),
                    digest: hex::encode(attestation.attestation.digest().as_bytes()),
                    prev_digest: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    root: hex::encode(attestation.attestation.root),
                },
            }),
        );

        assert!(!result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(result.is_checkpoint_in_range());
        assert!(result.get_unelected_attestors().is_empty());
        assert!(!result.merkle_roots_match());
        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
        assert!(result.last_attestation_digest_matches_attestation_digest_in_graphql());
        assert!(result.last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql());
        assert!(result.last_attesation_root_matches_attestation_root_in_graphql());
    }
    #[test]
    fn test_checkpoint_not_in_range_only() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        // Match the merkle root to make merkle_roots_match() pass
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: false,
        };
        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
            Some(vec![AccountId32::from([0u8; 32])]),
            Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "50".to_string(),
                    last_attested_digest: hex::encode(attestation.attestation.digest().as_bytes()),
                },
                attestation_node: AttestationNode {
                    header_number: "100".to_string(),
                    digest: hex::encode(attestation.attestation.digest().as_bytes()),
                    prev_digest: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    root: hex::encode(attestation.attestation.root),
                },
            }),
        );

        assert!(!result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(result.merkle_roots_match());
        assert!(result.get_unelected_attestors().is_empty());
        assert!(!result.is_checkpoint_in_range());
        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
        assert!(result.last_attestation_digest_matches_attestation_digest_in_graphql());
        assert!(result.last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql());
        assert!(result.last_attesation_root_matches_attestation_root_in_graphql());
    }

    #[test]
    fn test_unelected_attestors_only() {
        let attestation = SignedAttestation {
            attestation: Attestation {
                chain_key: 1,
                header_number: 100,
                header_hash: H256::zero(),
                root: [1u8; 32],
                prev_digest: Some(H256::zero()),
            },
            signature: [0u8; 96],
            attestors: vec![AccountId32::from([0u8; 32]), AccountId32::from([2u8; 32])],
        };
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };
        let maybe_elected_attestors = Some(vec![AccountId32::from([0u8; 32])]);

        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
            maybe_elected_attestors.clone(),
            Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "50".to_string(),
                    last_attested_digest: hex::encode(attestation.attestation.digest().as_bytes()),
                },
                attestation_node: AttestationNode {
                    header_number: "100".to_string(),
                    digest: hex::encode(attestation.attestation.digest().as_bytes()),
                    prev_digest: "0000000000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    root: hex::encode(attestation.attestation.root),
                },
            }),
        );

        let unelected = result.get_unelected_attestors();
        assert!(!result.is_block_height_exceeded());
        assert!(result.header_hash_matches());
        assert!(result.merkle_roots_match());
        assert!(result.is_checkpoint_in_range());
        assert_eq!(unelected, vec![AccountId32::from([2u8; 32])]);
        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
        assert!(result.last_attestation_digest_matches_attestation_digest_in_graphql());
        assert!(result.last_attestation_prev_digest_matches_attestation_prev_digest_in_graphql());
        assert!(result.last_attesation_root_matches_attestation_root_in_graphql());
    }

    #[test]
    fn test_attestation_signers_are_elected() {
        let attestation = dummy_attestation(100, [1u8; 32]);
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };
        let maybe_elected_attestors = Some(vec![
            AccountId32::from([0u8; 32]),
            AccountId32::from([1u8; 32]),
        ]);

        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
            maybe_elected_attestors.clone(),
            None,
        );

        let unelected = result.get_unelected_attestors();
        assert!(unelected.is_empty());
    }

    #[test]
    fn test_attestation_signers_are_not_elected() {
        let attestation = SignedAttestation {
            attestation: Attestation {
                chain_key: 1,
                header_number: 100,
                header_hash: H256::zero(),
                root: [1u8; 32],
                prev_digest: None,
            },
            signature: [0u8; 96],
            attestors: vec![AccountId32::from([0u8; 32]), AccountId32::from([2u8; 32])],
        };
        let latest_ethereum_block_number = 100;
        let fetched_ethereum_block_number_by_hash = Some(100u64.into());
        let calculated_ethereum_block_merkle_root = hex::encode([1u8; 32]);
        let checkpoint_created_in_range_checker = CheckpointCreatedWithinRangeResult {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number: 100,
            checkpoint_created_within_range: true,
        };
        let maybe_elected_attestors = Some(vec![
            AccountId32::from([0u8; 32]),
            AccountId32::from([1u8; 32]),
        ]);

        let result = compute_attestation_check_result(
            &attestation,
            latest_ethereum_block_number,
            &calculated_ethereum_block_merkle_root,
            fetched_ethereum_block_number_by_hash,
            checkpoint_created_in_range_checker,
            maybe_elected_attestors.clone(),
            None,
        );

        let unelected = result.get_unelected_attestors();
        assert_eq!(unelected, vec![AccountId32::from([2u8; 32])]);
    }

    #[test]
    fn test_graphql_attestation_checkpoint_and_attestation_found() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 10,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(10, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 5,
                latest_ethereum_block_number: 10,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "5".to_string(),
                    last_attested_digest: hex::encode(
                        dummy_attestation(10, [0u8; 32])
                            .attestation
                            .digest()
                            .as_bytes(),
                    ),
                },
                attestation_node: AttestationNode {
                    header_number: "10".to_string(),
                    digest: hex::encode(
                        dummy_attestation(10, [0u8; 32])
                            .attestation
                            .digest()
                            .as_bytes(),
                    ),
                    prev_digest: "0x4".to_string(),
                    root: hex::encode(dummy_attestation(10, [0u8; 32]).attestation.root),
                },
            }),
        };

        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
    }

    #[test]
    fn test_graphql_attestation_checkpoint_and_attestation_not_found() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(10, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 10,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "0".to_string(),
                    last_attested_digest: "0".to_string(),
                },
                attestation_node: AttestationNode {
                    header_number: "0".to_string(),
                    digest: "0".to_string(),
                    prev_digest: "0x4".to_string(),
                    root: hex::encode(dummy_attestation(10, [0u8; 32]).attestation.root),
                },
            }),
        };
        assert!(!result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            !result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
    }

    #[test]
    fn test_graphql_attestation_checkpoint_found_and_attestation_not_found() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(10, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 10,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "10".to_string(),
                    last_attested_digest: hex::encode(
                        dummy_attestation(10, [0u8; 32])
                            .attestation
                            .digest()
                            .as_bytes(),
                    ),
                },
                attestation_node: AttestationNode {
                    header_number: "0".to_string(),
                    digest: "0".to_string(),
                    prev_digest: "0x4".to_string(),
                    root: hex::encode(dummy_attestation(10, [0u8; 32]).attestation.root),
                },
            }),
        };

        assert!(result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            !result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
    }

    #[test]
    fn test_graphql_attestation_checkpoint_not_found_and_attestation_found() {
        let result = AttestationCheckResult {
            attestation_info: AttestationInfo {
                attestor_best_block_number: 0,
                attestation_merkle_root: String::new(),
                signed_attestation: dummy_attestation(10, [0u8; 32]),
            },
            block_height_diff: 0,
            ethereum_block_info: EthereumBlockInfo {
                latest_ethereum_block_number: 0,
                calculated_ethereum_block_merkle_root: String::new(),
                fetched_ethereum_block_number_by_hash: None,
            },
            check_point_created_in_range_checker: CheckpointCreatedWithinRangeResult {
                last_checkpoint_block_number: 10,
                latest_ethereum_block_number: 0,
                checkpoint_created_within_range: true,
            },
            maybe_elected_attestors: None,
            graphql_attestation_check_result: Some(GraphQLAttestationCheckResult {
                checkpoint_chain_node: AttestationCheckpointChainNode {
                    checkpoint_number: "0".to_string(),
                    last_attested_digest: "0".to_string(),
                },
                attestation_node: AttestationNode {
                    header_number: "10".to_string(),
                    digest: hex::encode(
                        dummy_attestation(10, [0u8; 32])
                            .attestation
                            .digest()
                            .as_bytes(),
                    ),
                    prev_digest: "0x4".to_string(),
                    root: hex::encode(dummy_attestation(10, [0u8; 32]).attestation.root),
                },
            }),
        };
        assert!(!result.last_checkpoint_header_matches_checkpoint_header_in_graphql());
        assert!(
            result.last_attestation_header_number_matches_attestation_header_number_in_graphql()
        );
    }
}
