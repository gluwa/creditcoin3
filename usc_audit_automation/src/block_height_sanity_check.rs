use anyhow::{Context, Result};
use ethers_core::types::U64;
use ethers_providers::{Http, Middleware, Provider};
use futures::future::FutureExt;
use mockall::{automock, predicate::*};
use reqwest::Client;
use std::convert::TryFrom;
use tracing::info;

use attestor_primitives::SignedAttestation;
use sp_core::H256;
use subxt::utils::AccountId32;

use cc_client::{Client as USCClient, Error as USCError};

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
}

impl EthereumProvider for Provider<Http> {
    fn fetch_block_number(&self) -> BoxFutureResult<'_, U64> {
        (async move {
            let block_number = Middleware::get_block_number(&self).await?;
            Ok(Some(block_number))
        })
        .boxed()
    }
}

pub async fn get_attestor_best_block_height(
    usc_client: &impl UniversalSmartContractProvider,
    target: &NetworkTarget,
) -> Result<u64> {
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

    Ok(signed_attestation.header_number())
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

pub async fn check_best_block_height_diff(config: &SanitiesConfigFile) -> Result<()> {
    let client = Client::new();

    for target in &config.targets {
        info!(
            "Started attestation network height diff check for: {}",
            target.usc_network_name
        );

        // Create provider
        let usc_client = USCClientWrapper(
            USCClient::new(target.usc_rpc_url.clone(), &target.usc_account_mnemonic).await?,
        );
        let provider = Provider::<Http>::try_from(target.ethereum_rpc_url.clone())
            .context("failed to create rpc provider")?;

        // Get last attested block
        let attestor_best_attestor_block =
            get_attestor_best_block_height(&usc_client, target).await?;
        info!("Attestor best block  {:?}\n", attestor_best_attestor_block);

        // Fetch current source chain block
        let latest_eth_block_number = get_ethereum_current_block_number(&provider).await?;
        info!("Ethereum best block  {:?}\n", latest_eth_block_number);

        let (primary_message, secondary_message) = create_json_message(
            target.clone(),
            attestor_best_attestor_block,
            latest_eth_block_number,
            config.slack_alert_group.clone(),
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
                .context("failed to send slack secondary json message")?;

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
