use anyhow::{Context, Result};
use attestor_primitives::SignedAttestation;
use cc_client::Client as USCClient;
use eth::{self, Client as EthClient};
use hex;
use reqwest::Client;
use sp_core::H256;
use subxt::utils::AccountId32;
use tracing::info;

use crate::{
    calculate_merkle_root, create_json_message::create_json_message, EthereumProvider,
    NetworkTarget, SanitiesConfigFile, USCClientWrapper, UniversalSmartContractProvider,
};

pub(crate) async fn get_attestor_latest_attestation_data(
    usc_client: &impl UniversalSmartContractProvider,
    target: &NetworkTarget,
) -> Result<SignedAttestation<H256, AccountId32>> {
    let chain_key = target.chain_key;

    // Fetch the last digest for this chain
    let last_digest = usc_client
        .fetch_last_digest(chain_key)
        .await?
        .with_context(|| format!("No last digest found for chain key {chain_key}"))?;

    // Get the signed attestation corresponding to the digest
    let signed_attestation = usc_client
        .get_attestation_by_digest(chain_key, last_digest)
        .await?
        .context("No attestation found for the given digest")?;

    Ok(signed_attestation)
}

pub(crate) async fn get_ethereum_current_block_number(
    provider: &impl EthereumProvider,
) -> Result<u64> {
    // Query latest block number
    let ethereum_block_number = provider
        .fetch_block_number()
        .await
        .context("failed to get ethereum block number")?;

    let ethereum_block_u64: u64 = ethereum_block_number
        .context("No block number returned from the provider")?
        .as_u64();

    Ok(ethereum_block_u64)
}

pub async fn run_attestation_sanity_checks(config: &SanitiesConfigFile) -> Result<()> {
    let client: Client = Client::new();

    for target in &config.targets {
        info!(
            "Started attestation network sanity check for: {}",
            target.usc_network_name
        );

        // Create USC and Ethereum clients
        let usc_client = USCClientWrapper(
            USCClient::new(target.usc_rpc_url.clone(), &target.usc_account_mnemonic).await?,
        );
        let eth_client: EthClient = EthClient::new(&target.ethereum_rpc_url, None).await?;

        // Get last signed attestation
        let latest_signed_attestation =
            get_attestor_latest_attestation_data(&usc_client, target).await?;

        // Fetch latest ethereum chain block number
        let latest_eth_block_number = get_ethereum_current_block_number(&eth_client).await?;
        info!("Ethereum best block  {:?}\n", latest_eth_block_number);

        // Get Ethereum block number using attestation header hash
        let eth_header_hash = ethers_core::types::H256::from_slice(
            latest_signed_attestation.attestation.header_hash.as_bytes(),
        );
        let fetched_ethereum_block_number_by_hash = eth_client
            .fetch_block_by_hash(eth_header_hash)
            .await
            .context("failed to fetch ethereum block by hash")?;
        info!(
            "Ethereum block by hash  {:?}\n",
            fetched_ethereum_block_number_by_hash
        );

        // Calculate merkle root from ethereum block number in attestation
        let calculated_ethereum_block_root = calculate_merkle_root(
            &eth_client,
            latest_signed_attestation.attestation.header_number(),
        )
        .await?;
        let ethereum_block_calculated_merkle_root =
            format!("0x{}", hex::encode(calculated_ethereum_block_root));
        info!(
            "Ethereum block calculated merkle root  {:?}\n",
            ethereum_block_calculated_merkle_root
        );

        let check_point_created_within_range_checker =
            crate::check_attestation_checkpoint_created_within_block_interval_range(
                &usc_client,
                target.chain_key,
                latest_eth_block_number,
            )
            .await?;
        info!(
            "Checkpoint created within block interval range: {}",
            check_point_created_within_range_checker.checkpoint_created_within_range
        );

        let attestation_check_result =
            crate::attestation_check_result::compute_attestation_check_result(
                &latest_signed_attestation,
                latest_eth_block_number,
                &ethereum_block_calculated_merkle_root,
                fetched_ethereum_block_number_by_hash,
                check_point_created_within_range_checker,
            );

        let (primary_message, secondary_message) = create_json_message(
            target.clone(),
            attestation_check_result,
            &config.slack_alert_group,
        );

        info!(
            "{}\n{}",
            primary_message,
            secondary_message.clone().unwrap_or_default()
        );

        let response = client
            .post(config.slack_webhook_url.clone())
            .json(&primary_message)
            .send()
            .await
            .context("failed to send slack primary json message")?;

        anyhow::ensure!(
            response.status().is_success(),
            "failed to submit primary slack API call"
        );

        info!("Secondary message: {:?}", secondary_message);
        if let Some(message) = secondary_message {
            let response = client
                .post(config.slack_webhook_url.clone())
                .json(&message)
                .send()
                .await
                .context("failed to submit secondary slack API call")?;

            anyhow::ensure!(
                response.status().is_success(),
                "failed to submit secondary slack API call"
            );
        }

        info!(
            "Completed attestation height diff check for  {:?}\n{}",
            target.usc_network_name, primary_message
        );
    }

    Ok(())
}
