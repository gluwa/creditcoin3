use anyhow::{Context, Result};
use attestor_primitives::SignedAttestation;
use cc_client::{Client as USCClient, Error as USCError};
use eth::{self, AlloyB256, Client as EthClient, OrderedBlock};
use ethers_core::types::U64;
use futures::future::FutureExt;
use hex;
use mmr::traits::MerkleTreeTrait;
use mockall::{automock, predicate::*};
use reqwest::Client;
use sp_core::H256;
use subxt::utils::AccountId32;
use tracing::info;

use crate::{
    create_json_message::create_json_message, BoxFutureResult, NetworkTarget, SanitiesConfigFile,
};

pub trait UniversalSmartContractProvider {
    fn fetch_last_digest(&self, chain_key: u64) -> BoxFutureResult<'_, H256>;
    fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> BoxFutureResult<'_, SignedAttestation<H256, AccountId32>, USCError>;
}

pub struct USCClientWrapper(USCClient);
impl UniversalSmartContractProvider for USCClientWrapper {
    fn fetch_last_digest(&self, chain_key: u64) -> BoxFutureResult<'_, H256> {
        (async move {
            self.0
                .fetch_last_digest(chain_key)
                .await
                .map_err(anyhow::Error::from)
        })
        .boxed()
    }

    fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> BoxFutureResult<'_, SignedAttestation<H256, AccountId32>, USCError> {
        (async move { self.0.get_attestation_by_digest(chain_key, digest).await }).boxed()
    }
}
#[automock]
pub trait EthereumProvider {
    fn fetch_block_number(&self) -> BoxFutureResult<'_, U64>;
    fn fetch_block_by_hash(&self, block_hash: ethers_core::types::H256)
        -> BoxFutureResult<'_, U64>;
    fn get_block_by_number(&self, block_number: u64) -> BoxFutureResult<'_, OrderedBlock>;
}

impl EthereumProvider for EthClient {
    fn fetch_block_number(&self) -> BoxFutureResult<'_, U64> {
        (async move {
            let block_number = self.get_last_block().await?;
            Ok(Some(U64::from(block_number)))
        })
        .boxed()
    }
    fn fetch_block_by_hash(
        &self,
        block_hash: ethers_core::types::H256,
    ) -> BoxFutureResult<'_, U64> {
        (async move {
            let block_number = self
                .get_block_number_by_hash(AlloyB256::from_slice(block_hash.as_bytes()))
                .await?;

            Ok(Some(U64::from(block_number)))
        })
        .boxed()
    }
    fn get_block_by_number(&self, block_number: u64) -> BoxFutureResult<'_, OrderedBlock> {
        (async move {
            let ordered_block = self.get_block(block_number).await?;
            Ok(Some(ordered_block))
        })
        .boxed()
    }
}

pub async fn get_attestor_latest_attestation_data(
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

pub async fn get_ethereum_current_block_number(provider: &impl EthereumProvider) -> Result<u64> {
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

pub async fn calculate_merkle_root(
    eth_client: &impl EthereumProvider,
    block_number: u64,
) -> Result<[u8; 32]> {
    let ordered_block = eth_client
        .get_block_by_number(block_number)
        .await?
        .context("Failed to get block")?;
    let merkle_tree = eth::starknet_pedersen_mmr(&ordered_block);
    Ok(merkle_tree.root().0.to_bytes_be())
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

        let (primary_message, secondary_message) = create_json_message(
            target.clone(),
            latest_signed_attestation,
            latest_eth_block_number,
            ethereum_block_calculated_merkle_root,
            fetched_ethereum_block_number_by_hash,
            config.slack_alert_group.clone(),
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
            target.usc_network_name, primary_message
        );
    }

    Ok(())
}
