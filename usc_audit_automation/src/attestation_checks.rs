use crate::{
    attestation_check_result::{compute_attestation_check_result, AttestationCheckContext},
    calculate_merkle_root, check_attestation_checkpoint_created_within_block_interval_range,
    clients::usc::decode::{fetch_genesis_block_dynamic, DecodedSignedAttestation},
    continuity_proofs::validate_continuity_proof,
    create_json_message::create_json_message,
    ethereum_rpc::get_ethereum_rpc_url_from_chain_cache,
    fetch_chains_json, get_graphql_attestation_check_result, ChainCache, EthereumProvider,
    SanitiesConfigFile, SupportedChain, SupportedChainInfo, USCClient, USCClientWrapper,
    UniversalSmartContractProvider,
};
use anyhow::{Context, Result};
use attestor_primitives::Digest;
use eth::Client as EthClient;
use hex;
use parity_scale_codec::Decode;
use reqwest::Client;
use subxt::dynamic::{storage, Value};
use subxt::utils::AccountId32;
use tracing::{error, info};

pub(crate) async fn get_attestor_latest_attestation_data(
    usc_client: &impl UniversalSmartContractProvider,
    last_digest: Digest,
    chain_key: u64,
) -> Result<DecodedSignedAttestation> {
    // Get the signed attestation corresponding to the digest
    let decoded_signed_attestation = usc_client
        .get_attestation_by_digest(chain_key, last_digest)
        .await?
        .context("No attestation found for the given digest")?;

    info!("Decoded signed attestation for chain key {chain_key} {decoded_signed_attestation:?}");
    Ok(decoded_signed_attestation)
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
    let chain_cache = ChainCache::from_chains(chain_info.context("No chain info found")?);

    let usc_rpc_url = &config.usc_rpc_url;
    info!("Connecting to USC RPC at {usc_rpc_url}");
    // Create USC client
    let usc_client =
        USCClientWrapper(USCClient::new(&config.usc_rpc_url, &config.usc_account_mnemonic).await?);

    // Iterate over supported chains
    let address = storage("SupportedChains", "SupportedChains", vec![]);
    info!("Queried supported chains storage address {:?}", &address);

    let mut supported_chains_iter = usc_client
        .0
        .api()
        .storage()
        .at_latest()
        .await?
        .iter(address)
        .await?;

    while let Some(Ok(kv)) = supported_chains_iter.next().await {
        let bytes = kv.value.encoded();
        let supported_chain = SupportedChain::decode(&mut &bytes[..])?;
        let chain_id = supported_chain.chain_id;
        let chain_name = match String::from_utf8(supported_chain.chain_name.clone()) {
            Ok(name) => name,
            Err(e) => {
                tracing::warn!(
                    chain_id,
                    error = %e,
                    "Invalid UTF-8 in chain_name; using empty string"
                );
                String::new()
            }
        };

        let chain_key  = usc_client
            .0
            .get_chain_key(supported_chain.chain_id, supported_chain.chain_name)
            .await?
            .with_context(|| format!(
                "Failed to get chain key for supported chain {{ id: {chain_id}, name: {chain_name} }}",
            ))?;

        let genesis_block_number =
            fetch_genesis_block_dynamic(usc_client.0.api(), chain_key).await?;
        info!(
            "Fetched genesis block number for chain key {chain_key}: {:?}",
            genesis_block_number
        );

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
        info!(
            "Using Ethereum RPC URL: {}",
            crate::ethereum_rpc::redact_api_key_from_url(&eth_rpc_url)
        );

        // Create Ethereum client
        let eth_client: EthClient = EthClient::new(&eth_rpc_url, None).await?;

        let last_digest = match usc_client.fetch_last_digest(chain_key).await {
            Ok(Some(digest)) => digest,
            Ok(None) => {
                error!(
                    "❌ No last digest found for chain key {chain_key}. Skipping sanity checks for this chain."
                );
                continue;
            }
            Err(e) => {
                error!(
                    "Error fetching last digest for chain key {chain_key}: {}",
                    e
                );
                continue;
            }
        };

        info!(
            "Fetched last digest for chain key {chain_key}: 0x{:x}",
            last_digest
        );

        let latest = match get_attestor_latest_attestation_data(
            &usc_client,
            last_digest,
            supported_chain_info.chain_key,
        )
        .await
        {
            Ok(latest) => latest, // latest is now: DecodedSignedAttestation
            Err(e) => {
                error!(
                    "No signed attestation found for {} with chain key: {}. \
             Skipping sanity checks. Error: {}",
                    supported_chain_info.chain_name, supported_chain_info.chain_key, e
                );
                continue;
            }
        };

        let latest_signed_attestation = latest.value; // SignedAttestation
        let proof_status = latest.proof_status;
        let continuity_proof_is_valid = if let Some(genesis_block_number) = genesis_block_number {
            // validate_continuity_proof handles both genesis (prev_digest = None) and non-genesis cases
            validate_continuity_proof(
                &usc_client,
                &latest_signed_attestation, // SignedAttestation
                genesis_block_number,
                latest_signed_attestation.attestation.prev_digest, // previously finalized digest
                proof_status,
            )
            .await
        } else {
            false
        };

        info!(
            "Continuity proof validation result: {}",
            continuity_proof_is_valid
        );

        info!(
            "Latest signed attestation fetched: {:?}",
            latest_signed_attestation.attestation.root
        );
        info!("Last signed attestation: {:?}", latest_signed_attestation);

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
        let block_number = latest_signed_attestation.attestation.header_number();
        info!("Ethereum block calculated merkle root 0x{ethereum_block_calculated_merkle_root} for block number: {block_number}");

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

        let elected_attestors_storage_query = storage(
            "Attestation",     // pallet name exactly as shown in metadata
            "ActiveAttestors", // storage item name exactly as shown in metadata
            vec![Value::from(supported_chain_info.chain_key)],
        );
        let maybe_elected_attestors = usc_client
            .0
            .api()
            .storage()
            .at_latest()
            .await?
            .fetch(&elected_attestors_storage_query)
            .await?;

        let maybe_elected_attestors: Option<Vec<AccountId32>> = maybe_elected_attestors
            .map(|val| val.as_type::<Vec<AccountId32>>())
            .transpose()?; // converts Option<Result<_>> -> Result<Option<_>>

        let last_checkpoint_block_number =
            check_point_created_within_range_checker.last_checkpoint_block_number;

        let attestation_in_graphql_result = get_graphql_attestation_check_result(
            &client,
            supported_chain_info.chain_key,
            latest_signed_attestation.attestation.header_number(),
            last_checkpoint_block_number,
            config.usc_attestations_graphql_url.clone(),
        )
        .await?;
        info!(
            "Last checkpoint found in GraphQL: {:?}",
            attestation_in_graphql_result
        );

        let attestation_check_context = AttestationCheckContext {
            latest_signed_attestation: &latest_signed_attestation,
            latest_ethereum_block_number: latest_eth_block_number,
            calculated_ethereum_block_merkle_root: &ethereum_block_calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
            check_point_created_in_range_checker: check_point_created_within_range_checker,
            maybe_elected_attestors,
            graphql_attestation_check_result: attestation_in_graphql_result,
            continuity_proof_is_valid,
        };
        let attestation_check_result = compute_attestation_check_result(&attestation_check_context);

        let (primary_message, secondary_message) = create_json_message(
            &supported_chain_info,
            attestation_check_result,
            &config.usc_network_name,
            &config.slack_alert_group,
        );

        info!(
            "Primary: {}\nSecondary: {}",
            primary_message,
            secondary_message.clone().unwrap_or_default()
        );

        let response = client
            .post(config.slack_webhook_url.clone())
            .json(&primary_message)
            .send()
            .await
            .context("failed to send slack primary json message")?;

        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            error!(
                status = %status,
                body = %body,
                "Slack primary API call failed"
            );
            anyhow::bail!("Failed to submit primary slack API call: {status}");
        }

        if let Some(message) = secondary_message {
            let response = client
                .post(config.slack_webhook_url.clone())
                .json(&message)
                .send()
                .await
                .context("failed to submit secondary slack API call")?;

            let status = response.status();
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                error!(
                    status = %status,
                    body = %body,
                    "Slack secondary API call failed"
                );
                anyhow::bail!("Failed to submit secondary slack API call: {status}");
            }
        }

        info!(
            "Completed attestation height diff check for  {:?}\n{}",
            supported_chain_info.chain_name, primary_message
        );
    }
    Ok(())
}
