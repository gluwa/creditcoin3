use crate::{
    attestation_check_result::compute_attestation_check_result,
    attestation_checks::{get_attestor_latest_attestation_data, get_ethereum_current_block_number},
    calculate_merkle_root, calculate_usc_and_source_chain_block_diff,
    create_json_message::create_json_message,
    BoxFutureResult, CheckPointCreatedWithinRangeChecker, MockEthereumProvider,
    UniversalSmartContractProvider,
};
use anyhow::Result;
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use ethers::types::U64;
use futures::future::FutureExt;
use hex_literal::hex;
use serde_json::json;
use sp_core::H256;
use subxt::utils::AccountId32;

use eth::OrderedBlock;
use mockall::predicate::*;
use std::sync::Arc;

pub struct MockUscRpcClientOk;

impl MockUscRpcClientOk {
    pub fn new() -> Self {
        MockUscRpcClientOk
    }
}

impl UniversalSmartContractProvider for MockUscRpcClientOk {
    fn fetch_last_digest(&self, _chain_key: u64) -> BoxFutureResult<'_, H256> {
        (async move { Ok(Some(H256::from_slice(&[0u8; 32]))) }).boxed()
    }

    fn get_attestation_by_digest(
        &self,
        _chain_key: u64,
        _digest: H256,
    ) -> BoxFutureResult<'_, SignedAttestation<H256, AccountId32>, cc_client::Error> {
        (async move {
            Ok(Some(SignedAttestation {
                attestation: attestor_primitives::Attestation {
                    chain_key: 2,
                    header_number: 123,
                    header_hash: H256::from_slice(&[0u8; 32]),
                    root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
                    prev_digest: None,
                },
                signature: [0u8; 96],
                attestors: vec![],
            }))
        })
        .boxed()
    }
    fn get_attestation_interval(&self, _chain_key: u64) -> BoxFutureResult<'_, u64> {
        (async move { Ok(Some(5)) }).boxed()
    }
    fn get_checkpoint_interval(&self, _chain_key: u64) -> BoxFutureResult<'_, u32> {
        (async move { Ok(Some(10)) }).boxed()
    }
    fn get_last_attestation_checkpoint(
        &self,
        _chain_key: u64,
    ) -> BoxFutureResult<'_, AttestationCheckpoint> {
        (async move {
            Ok(Some(AttestationCheckpoint {
                block_number: 100,
                digest: H256::from_slice(&[0u8; 32]),
            }))
        })
        .boxed()
    }
    fn get_attestation_vote_acceptance_window(&self, _chain_key: u64) -> BoxFutureResult<'_, u64> {
        (async move { Ok(Some(2_u64)) }).boxed()
    }
}

struct MockUscRpcClientError;
impl MockUscRpcClientError {
    pub fn new() -> Self {
        MockUscRpcClientError
    }
}

impl UniversalSmartContractProvider for MockUscRpcClientError {
    fn fetch_last_digest(&self, _chain_key: u64) -> BoxFutureResult<'_, H256> {
        (async move { Ok(None) }).boxed()
    }

    fn get_attestation_by_digest(
        &self,
        _chain_key: u64,
        _digest: H256,
    ) -> BoxFutureResult<'_, SignedAttestation<H256, AccountId32>, cc_client::Error> {
        (async move { Ok(None) }).boxed()
    }
    fn get_attestation_interval(&self, _chain_key: u64) -> BoxFutureResult<'_, u64> {
        (async move { Ok(Some(5_u64)) }).boxed()
    }
    fn get_checkpoint_interval(&self, _chain_key: u64) -> BoxFutureResult<'_, u32> {
        (async move { Ok(Some(10_u32)) }).boxed()
    }
    fn get_last_attestation_checkpoint(
        &self,
        _chain_key: u64,
    ) -> BoxFutureResult<'_, AttestationCheckpoint> {
        (async move { Ok(None) }).boxed()
    }
    fn get_attestation_vote_acceptance_window(&self, _chain_key: u64) -> BoxFutureResult<'_, u64> {
        (async move { Ok(Some(2_u64)) }).boxed()
    }
}

#[tokio::test]
async fn get_attestor_best_block_height_fetches_best_block_correctly() -> Result<()> {
    let client = MockUscRpcClientOk::new();
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "test_url".to_string(),
        usc_account_mnemonic: "".to_string(),
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
        chain_key: 2,
    };

    // Call mock get_attestor_best_block_height
    let signed_attestation = get_attestor_latest_attestation_data(&client, target).await?;
    let expected_best_block = 123_u64;

    assert_eq!(
        expected_best_block,
        signed_attestation.attestation.header_number
    );

    Ok(())
}

#[tokio::test]
async fn get_attestor_best_block_height_returns_an_error_when_no_best_block_found() -> Result<()> {
    let client = MockUscRpcClientError::new();
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "test_url".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };

    // Call mock get_attestor_best_block_height
    let result = get_attestor_latest_attestation_data(&client, target).await;

    assert!(result.is_err(), "Expected an error but got Ok");

    let err = result.unwrap_err();
    assert!(
        err.to_string()
            .contains("No last digest found for chain key 2"),
        "Unexpected error: {err}"
    );

    Ok(())
}

#[tokio::test]
async fn get_ethereum_current_block_number_correctly_gets_block_number() -> Result<()> {
    let mut mock = MockEthereumProvider::new(); // generated by mock call

    // Mock ethereum trait return value
    mock.expect_fetch_block_number()
        .times(1)
        .returning(|| (async { Ok(Some(42.into())) }).boxed());

    // Call mock get_ethereum_current_block_number
    let eth_block_number = get_ethereum_current_block_number(&mock).await?;
    let expected_block_number = 42_u64;
    assert_eq!(expected_block_number, eth_block_number);

    Ok(())
}

#[tokio::test]
async fn get_ethereum_current_block_number_correctly_handles_an_error() -> Result<()> {
    let mut mock = MockEthereumProvider::new();

    mock.expect_fetch_block_number()
        .returning(|| (async { Err(anyhow::anyhow!("RPC error: connection dropped")) }).boxed());

    let result = get_ethereum_current_block_number(&mock).await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert_eq!(err.to_string(), "failed to get ethereum block number");
    let mut chain = err.chain();
    chain.next();

    let original_error = chain.next().unwrap();
    assert_eq!(original_error.to_string(), "RPC error: connection dropped");

    Ok(())
}

#[test]
fn calculate_usc_and_source_chain_block_diff_returns_a_positive_value_when_source_chain_is_ahead_of_usc_chain(
) {
    let attestor_block_height = 900_020_u64;
    let source_chain_block_height = 900_050_u64;
    let expected_result = 30_i128;

    let result =
        calculate_usc_and_source_chain_block_diff(attestor_block_height, source_chain_block_height);
    assert_eq!(expected_result, result);
}

#[test]
fn calculate_usc_and_source_chain_block_diff_returns_a_negative_value_if_source_chain_is_behind_usc_chain(
) {
    let attestor_block_height = 900_010_u64;
    let source_chain_block_height = 900_000_u64;
    let expected_result = -10_i128;

    let result =
        calculate_usc_and_source_chain_block_diff(attestor_block_height, source_chain_block_height);
    assert_eq!(expected_result, result);
}
#[test]
fn create_message_returns_chain_status_and_group_notification_when_slack_group_is_some_and_block_diff_check_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };

    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 100,
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let slack_alert_group = Some("S_TEST_GROUP".to_string());
    let calculated_ethereum_block_merkle_root = format!(
        "0x{}",
        "736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"
    );
    let fetched_ethereum_block_number_by_hash = Some(U64::from(100u64));
    let latest_ethereum_block_number = 200_u64;

    let last_checkpoint_block_number = 50;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n❌ Attestation block heights diff: 100 (200|100)\n✅ Attestation header hash matches correct Ethereum block: (100|100)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        Some(json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":rotating_light:",
            "text": "<!subteam^S_TEST_GROUP> The following issues were detected:\nCurrent block difference exceeds threshold!"
        })),
    );
    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_group_notification_when_slack_group_is_some_and_header_hash_check_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };

    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 150, // block diff is 50 (passes)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let slack_alert_group = Some("S_TEST_GROUP".to_string());
    let calculated_ethereum_block_merkle_root = format!(
        "0x{}",
        "736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"
    );
    let fetched_ethereum_block_number_by_hash = Some(U64::from(100u64)); // mismatch: 100 != 150
    let latest_ethereum_block_number = 200_u64;

    let last_checkpoint_block_number = 180;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 50 (200|150)\n❌ Attestation header hash does not match correct Ethereum block: (100|150)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 20 (200|180)```"
        }),
        Some(json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":rotating_light:",
            "text": "<!subteam^S_TEST_GROUP> The following issues were detected:\nAttestation header hash does not match correct Ethereum block!"
        })),
    );

    let attestation_checks_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true, // checkpoint check passes
        },
    );

    let result = create_json_message(
        target.clone(),
        attestation_checks_result,
        &slack_alert_group,
    );

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_group_notification_when_slack_group_is_some_and_merkle_root_check_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };

    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 150, // block diff is 50 (passes)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let slack_alert_group = Some("S_TEST_GROUP".to_string());
    let calculated_ethereum_block_merkle_root = "0xsomecalculatedmerkle".to_string(); // mismatch
    let fetched_ethereum_block_number_by_hash = Some(U64::from(150u64)); // matches header_number
    let latest_ethereum_block_number = 200_u64;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 50 (200|150)\n✅ Attestation header hash matches correct Ethereum block: (150|150)\n❌ Calculated merkle root does not match attestation root: (0xsomecalculatedmerkle|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        Some(json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":rotating_light:",
            "text": "<!subteam^S_TEST_GROUP> The following issues were detected:\nCalculated merkle root does not match attestation root!"
        })),
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_no_group_notification_when_slack_group_is_some_and_all_checks_pass(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 170,
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root =
        "0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000".to_string();
    let fetched_ethereum_block_number_by_hash = Some(U64::from(170u64));
    let slack_alert_group = Some("TEST_GROUP".to_string());

    let last_checkpoint_block_number = 180;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 30 (200|170)\n✅ Attestation header hash matches correct Ethereum block: (170|170)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 20 (200|180)```"
        }),
        None,
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_group_notification_with_u_slack_id_when_a_check_fails() {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 100, // block diff is 100 (fails)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root =
        "0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000".to_string();
    let fetched_ethereum_block_number_by_hash = Some(U64::from(100u64));
    let slack_alert_group = Some("U_TEST_USER".to_string());

    let last_checkpoint_block_number = 50;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n❌ Attestation block heights diff: 100 (200|100)\n✅ Attestation header hash matches correct Ethereum block: (100|100)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        Some(json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":rotating_light:",
            "text": "<@U_TEST_USER> The following issues were detected:\nCurrent block difference exceeds threshold!"
        })),
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_no_group_notification_when_slack_group_is_none_and_all_checks_fail(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 140,
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root = "0xsomecalculatedmerkle".to_string();
    let fetched_ethereum_block_number_by_hash = Some(U64::from(100u64));
    let slack_alert_group = None;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n❌ Attestation block heights diff: 60 (200|140)\n❌ Attestation header hash does not match correct Ethereum block: (100|140)\n❌ Calculated merkle root does not match attestation root: (0xsomecalculatedmerkle|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n❌ Last checkpoint creation is outside checkpoint range: 150 (200|50)```"
        }),
        None,
    );

    let attestation_checks_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number: 50,
            latest_ethereum_block_number,
            checkpoint_created_within_range: false, // <-- Set to false so checkpoint check fails
        },
    );

    let result = create_json_message(
        target.clone(),
        attestation_checks_result,
        &slack_alert_group,
    );

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_no_group_notification_when_slack_group_is_none_and_block_diff_check_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 100, // block diff is 100 (fails)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root =
        "0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000".to_string();
    let fetched_ethereum_block_number_by_hash = Some(U64::from(100u64)); // matches header_number
    let slack_alert_group = None;

    let last_checkpoint_block_number = 50;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n❌ Attestation block heights diff: 100 (200|100)\n✅ Attestation header hash matches correct Ethereum block: (100|100)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        None,
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_no_group_notification_when_slack_group_is_none_and_header_hash_check_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 150, // block diff is 50 (passes)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root =
        "0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000".to_string();
    let fetched_ethereum_block_number_by_hash = Some(U64::from(100u64)); // mismatch: 100 != 150
    let slack_alert_group = None;

    let last_checkpoint_block_number = 50;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 50 (200|150)\n❌ Attestation header hash does not match correct Ethereum block: (100|150)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        None,
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_no_group_notification_when_slack_group_is_none_and_merkle_root_check_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 150, // block diff is 50 (passes)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root = "0xsomecalculatedmerkle".to_string(); // mismatch
    let fetched_ethereum_block_number_by_hash = Some(U64::from(150u64)); // matches header_number
    let slack_alert_group = None;

    let last_checkpoint_block_number = 50;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 50 (200|150)\n✅ Attestation header hash matches correct Ethereum block: (150|150)\n❌ Calculated merkle root does not match attestation root: (0xsomecalculatedmerkle|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        None,
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_a_chain_status_and_no_group_notification_when_slack_group_is_none_and_sanity_check_succeeds(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 150,
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let calculated_ethereum_block_merkle_root =
        "0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000".to_string();
    let fetched_ethereum_block_number_by_hash = Some(U64::from(150u64));
    let slack_alert_group = None;

    let last_checkpoint_block_number = 50;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 50 (200|150)\n✅ Attestation header hash matches correct Ethereum block: (150|150)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n✅ Last checkpoint creation is within checkpoint range: 150 (200|50)```"
        }),
        None,
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        calculated_ethereum_block_merkle_root.as_str(),
        fetched_ethereum_block_number_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: true,
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[test]
fn create_message_returns_chain_status_and_no_group_notification_when_slack_group_is_none_and_only_checkpoint_range_fails(
) {
    let target = &crate::NetworkTarget {
        usc_network_name: "TEST".to_string(),
        usc_rpc_url: "http://someuscmetricsurl.com/metrics".to_string(),
        usc_account_mnemonic: "".to_string(),
        chain_key: 2,
        ethereum_rpc_url: "http://someethrpcurl.com".to_string(),
    };
    let last_signed_attestation = SignedAttestation {
        attestation: attestor_primitives::Attestation {
            chain_key: 2,
            header_number: 150, // block diff is 50 (passes)
            header_hash: H256::from_slice(&[0u8; 32]),
            root: hex!("736f6d6563616c63756c617465646d65726b6c65000000000000000000000000"),
            prev_digest: None,
        },
        signature: [0u8; 96],
        attestors: vec![],
    };
    let latest_ethereum_block_number = 200_u64;
    let ethereum_current_block_calculated_merkle_root =
        "0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000".to_string();
    let eth_block_by_hash = Some(U64::from(150u64)); // matches header_number
    let slack_alert_group: Option<String> = None;

    // Set checkpoint values so the check fails (latest_ethereum_block_number - last_checkpoint_block_number > checkpoint_creation_range)
    let last_checkpoint_block_number = 10;

    let expected_result = (
        json!({
            "username": "usc-audit-automation",
            "icon_emoji": ":shield:",
            "text": "```⬛ TEST\n✅ Attestation block heights diff: 50 (200|150)\n✅ Attestation header hash matches correct Ethereum block: (150|150)\n✅ Calculated merkle root matches attestation root: (0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000|0x736f6d6563616c63756c617465646d65726b6c65000000000000000000000000)\n❌ Last checkpoint creation is outside checkpoint range: 190 (200|10)```"
        }),
        None,
    );

    let attestation_check_result = compute_attestation_check_result(
        &last_signed_attestation,
        latest_ethereum_block_number,
        &ethereum_current_block_calculated_merkle_root,
        eth_block_by_hash,
        CheckPointCreatedWithinRangeChecker {
            last_checkpoint_block_number,
            latest_ethereum_block_number,
            checkpoint_created_within_range: false, // fails
        },
    );

    let result = create_json_message(target.clone(), attestation_check_result, &slack_alert_group);

    assert_eq!(expected_result, result);
}

#[tokio::test]
async fn test_calculate_merkle_root_returns_expected_bytes() {
    // Prepare a test OrderedBlock and expected root
    let chain_id = 1u64;
    let number = 123u64;
    let hash = &[0u8; 32];
    let transactions = vec![];
    let receipts = vec![];
    let test_ordered_block =
        OrderedBlock::try_create(chain_id, number, hash.into(), transactions, receipts).unwrap();
    let test_ordered_block = Arc::new(test_ordered_block); // Wrap in Arc

    // Set up the mock client
    let mut mock_client = MockEthereumProvider::new();
    let block_clone = Arc::clone(&test_ordered_block);
    mock_client
        .expect_get_block_by_number()
        .with(eq(123u64))
        .returning(move |_| {
            let block = Arc::clone(&block_clone);
            (async move { Ok(Some((*block).clone())) }).boxed()
        });

    // Call the function
    let result = calculate_merkle_root(&mock_client, 123).await;

    assert!(result.is_ok());
    assert_eq!(result.unwrap().len(), 32);
}
