use anyhow::{Context, Result};
use attestor_primitives::SignedAttestation;
use cc_client::{
    self, cc3, cc3::runtime_types::supported_chains_primitives::SupportedChain, Client as USCClient,
};
use eth::{self, Client as EthClient};
use hex;
use reqwest::Client;
use sp_core::H256;
use subxt::utils::AccountId32;
use tracing::{error, info};

use crate::{
    attestation_check_result::compute_attestation_check_result, calculate_merkle_root,
    check_attestation_checkpoint_created_within_block_interval_range,
    create_json_message::create_json_message, ethereum_rpc::get_ethereum_rpc_url_from_chain_cache,
    fetch_chains_json, ChainCache, EthereumProvider, SanitiesConfigFile, SupportedChainInfo,
    USCClientWrapper, UniversalSmartContractProvider,
};

pub(crate) async fn get_attestor_latest_attestation_data(
    usc_client: &impl UniversalSmartContractProvider,
    chain_key: u64,
) -> Result<SignedAttestation<H256, AccountId32>> {
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

    // Fetch chain info and build cache
    let chain_info = fetch_chains_json(&client).await?;
    let chain_cache = ChainCache::from_chains(chain_info.expect("No chain info found"));

    // Create USC client
    let usc_client = USCClientWrapper(
        USCClient::new(&config.usc_rpc_url.clone(), &config.usc_account_mnemonic).await?,
    );

    // Iterate over supported chains
    let address = cc3::storage().supported_chains().supported_chains_iter();
    let mut supported_chains_iter = usc_client
        .0
        .api()
        .await?
        .storage()
        .at_latest()
        .await?
        .iter(address)
        .await?;

    while let Some(Ok(kv)) = supported_chains_iter.next().await {
        let supported_chain: SupportedChain = kv.value;
        let chain_name = String::from_utf8(supported_chain.chain_name.clone()).unwrap_or_default();
        let chain_id = supported_chain.chain_id;

        let chain_key = usc_client
            .0
            .get_chain_key(supported_chain.chain_id, supported_chain.chain_name)
            .await?
            .with_context(|| format!(
                "Failed to get chain key for supported chain {{ id: {chain_id}, name: {chain_name} }}",
            ))?;

        info!(
            "Started attestation network sanity check for chain key: {} - {}",
            chain_id, chain_name
        );

        let supported_chain_info = SupportedChainInfo {
            chain_id,
            chain_name,
            chain_key,
        };

        let maybe_eth_rpc_url = get_ethereum_rpc_url_from_chain_cache(
            chain_cache.clone(),
            &config.rpc_providers,
            &supported_chain_info,
        )
        .await;

        let Some(eth_rpc_url) = maybe_eth_rpc_url else {
            error!(
                "Failed to determine Ethereum RPC URL for chain: {}. Skipping sanity checks for this chain.",
                supported_chain_info.chain_name
            );
            continue;
        };
        info!("Using Ethereum RPC URL: {}", eth_rpc_url);

        // Create Ethereum client
        let eth_client: EthClient = EthClient::new(&eth_rpc_url, None).await?;

        // Get last signed attestation, continue if none found
        let Ok(latest_signed_attestation) =
            get_attestor_latest_attestation_data(&usc_client, supported_chain_info.chain_key).await
        else {
            error!(
                    "No signed attestation found for {} with chain key: {}. Skipping sanity checks for this chain.",
                    supported_chain_info.chain_name, supported_chain_info.chain_key
                );
            continue;
        };

        // Fetch latest ethereum chain block number
        let latest_eth_block_number = get_ethereum_current_block_number(&eth_client).await?;
        info!("Ethereum best block  {:?}", latest_eth_block_number);

        // Get Ethereum block number using attestation header hash
        let eth_header_hash = ethers_core::types::H256::from_slice(
            latest_signed_attestation.attestation.header_hash.as_bytes(),
        );
        let fetched_ethereum_block_number_by_hash = eth_client
            .fetch_block_by_hash(eth_header_hash)
            .await
            .context("failed to fetch ethereum block by hash")?;
        info!(
            "Ethereum block by hash  {:?}",
            fetched_ethereum_block_number_by_hash
        );

        // Calculate merkle root from ethereum block number in attestation
        let calculated_ethereum_block_root = calculate_merkle_root(
            &eth_client,
            latest_signed_attestation.attestation.header_number(),
        )
        .await?;

        let ethereum_block_calculated_merkle_root = hex::encode(calculated_ethereum_block_root);
        info!("Ethereum block calculated merkle root 0x{ethereum_block_calculated_merkle_root}\n");

        let check_point_created_within_range_checker =
            check_attestation_checkpoint_created_within_block_interval_range(
                &usc_client,
                supported_chain_info.chain_key,
                latest_eth_block_number,
            )
            .await?;
        info!(
            "Checkpoint created within block interval range: {}",
            check_point_created_within_range_checker.checkpoint_created_within_range
        );

        let elected_attestors_storage_query = cc_client::cc3::storage()
            .attestation()
            .active_attestors(supported_chain_info.chain_key);

        let maybe_elected_attestors = usc_client
            .0
            .api()
            .await?
            .storage()
            .at_latest()
            .await?
            .fetch(&elected_attestors_storage_query)
            .await?;

        let attestation_check_result = compute_attestation_check_result(
            &latest_signed_attestation,
            latest_eth_block_number,
            &ethereum_block_calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
            check_point_created_within_range_checker,
            maybe_elected_attestors,
        );

        let (primary_message, secondary_message) = create_json_message(
            &supported_chain_info,
            attestation_check_result,
            &config.usc_network_name,
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
            supported_chain_info.chain_name, primary_message
        );
    }
    Ok(())
}
