use anyhow::{Context, Result};
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::Client as USCClient;
use clap::Parser;
use eth::{self, AlloyB256, Client as EthClient, OrderedBlock};
use ethers::types::U64;
use mmr::traits::MerkleTreeTrait;
use mockall::{automock, predicate::*};
use reqwest::Client;
use serde::Deserialize;
use sp_core::H256;
use std::collections::HashMap;
use std::path::PathBuf;
use subxt::utils::AccountId32;
use tracing::info;
const MAX_ALLOWED_BLOCK_HEIGHT_DIFF: i128 = 50;
const CHAIN_LIST_URL: &str = "https://chainid.network/chains.json";

pub mod attestation_check_result;
pub mod attestation_checks;
mod create_json_message;
mod ethereum_rpc;
#[cfg(test)]
mod tests;

#[derive(Parser, Debug)]
#[command(name = "sanities checker")]
pub struct SanitiesChecker {
    #[arg(
        long,
        help = "Path to a TOML config file defining targets, polling interval, and Slack webhook."
    )]
    pub config_file: PathBuf,
}

#[derive(Debug, Deserialize, Clone)]
pub struct RpcProvider {
    name: String,
    api_key: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct SanitiesConfigFile {
    #[serde(default)]
    pub slack_webhook_url: String,
    #[serde(default)]
    pub slack_alert_group: Option<String>,
    #[serde(default)]
    pub log_verbose: bool,
    #[serde(default)]
    pub usc_network_name: String,
    #[serde(default)]
    pub usc_rpc_url: String,
    #[serde(default)]
    pub usc_account_mnemonic: String,
    #[serde(default)]
    pub rpc_providers: Vec<RpcProvider>,
    #[serde(default)]
    pub usc_graphql_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChainInfo {
    pub name: String,
    #[serde(rename = "chainId")]
    pub chain_id: u64,
    pub rpc: Vec<String>,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub network: Option<String>,
}

pub async fn fetch_chains_json(client: &Client) -> Result<Option<Vec<ChainInfo>>> {
    let response = client.get(CHAIN_LIST_URL).send().await?;
    let chains: Vec<ChainInfo> = response.json().await?;

    Ok(Some(chains))
}

pub struct SupportedChainInfo {
    pub chain_id: u64,
    pub chain_name: String,
    pub chain_key: u64,
}

#[derive(Debug, Clone)]
pub struct ChainCache {
    by_id: HashMap<u64, ChainInfo>,
    by_name: HashMap<String, ChainInfo>,
}

impl ChainCache {
    pub fn from_chains(chains: Vec<ChainInfo>) -> Self {
        let mut by_id = HashMap::new();
        let mut by_name = HashMap::new();

        for chain in chains {
            by_id.insert(chain.chain_id, chain.clone());
            by_name.insert(chain.name.to_lowercase(), chain);
        }

        Self { by_id, by_name }
    }

    pub fn get_by_id(&self, id: u64) -> Option<&ChainInfo> {
        self.by_id.get(&id)
    }

    pub fn get_by_name(&self, name: &str) -> Option<&ChainInfo> {
        self.by_name.get(&name.to_lowercase())
    }
}

#[automock]
pub(crate) trait EthereumProvider {
    async fn fetch_block_number(&self) -> Result<Option<U64>>;
    async fn fetch_block_by_hash(
        &self,
        block_hash: ethers_core::types::H256,
    ) -> Result<Option<U64>>;
    async fn get_block_by_number(&self, block_number: u64) -> Result<Option<OrderedBlock>>;
}
impl EthereumProvider for EthClient {
    async fn fetch_block_number(&self) -> Result<Option<U64>> {
        let block_number = self.get_last_block().await?;

        Ok(Some(U64::from(block_number)))
    }
    async fn fetch_block_by_hash(
        &self,
        block_hash: ethers_core::types::H256,
    ) -> Result<Option<U64>> {
        let block_number = self
            .get_block_number_by_hash(AlloyB256::from_slice(block_hash.as_bytes()))
            .await?;

        Ok(Some(U64::from(block_number)))
    }
    async fn get_block_by_number(&self, block_number: u64) -> Result<Option<OrderedBlock>> {
        let ordered_block = self.get_block(block_number).await?;

        Ok(Some(ordered_block))
    }
}

pub(crate) trait UniversalSmartContractProvider {
    async fn fetch_last_digest(&self, chain_key: u64) -> Result<Option<H256>>;
    async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>>;
    async fn get_last_attestation_checkpoint(
        &self,
        chain_key: u64,
    ) -> Result<Option<AttestationCheckpoint>>;
    async fn get_checkpoint_interval(&self, chain_key: u64) -> Result<Option<u32>>;
    async fn get_attestation_interval(&self, chain_key: u64) -> Result<Option<u64>>;
    async fn get_attestation_vote_acceptance_window(&self, chain_key: u64) -> Result<Option<u64>>;
}

pub struct USCClientWrapper(USCClient);
impl UniversalSmartContractProvider for USCClientWrapper {
    async fn fetch_last_digest(&self, chain_key: u64) -> Result<Option<H256>> {
        let last_digest = self.0.fetch_last_digest(chain_key).await?;

        Ok(last_digest)
    }

    async fn get_attestation_by_digest(
        &self,
        chain_key: u64,
        digest: H256,
    ) -> Result<Option<SignedAttestation<H256, AccountId32>>> {
        let signed_attestation = self.0.get_attestation_by_digest(chain_key, digest).await?;

        Ok(signed_attestation)
    }

    async fn get_last_attestation_checkpoint(
        &self,
        chain_key: u64,
    ) -> Result<Option<AttestationCheckpoint>> {
        let last_attestation_checkpoint = self.0.get_last_checkpoint(chain_key).await?;

        Ok(last_attestation_checkpoint)
    }
    async fn get_checkpoint_interval(&self, chain_key: u64) -> Result<Option<u32>> {
        let checkpoint_interval = self.0.chain_checkpoint_interval(chain_key).await?;

        Ok(checkpoint_interval)
    }
    async fn get_attestation_interval(&self, chain_key: u64) -> Result<Option<u64>> {
        let attestation_interval = self.0.chain_attestation_interval(chain_key).await?;

        Ok(attestation_interval)
    }
    async fn get_attestation_vote_acceptance_window(&self, chain_key: u64) -> Result<Option<u64>> {
        let vote_acceptance_window = self
            .0
            .get_attestation_vote_acceptance_window(chain_key)
            .await?;

        Ok(vote_acceptance_window)
    }
}

#[derive(Debug)]
pub struct CheckpointCreatedWithinRangeResult {
    last_checkpoint_block_number: u64,
    latest_ethereum_block_number: u64,
    checkpoint_created_within_range: bool,
}

pub(crate) async fn check_attestation_checkpoint_created_within_block_interval_range(
    client: &impl UniversalSmartContractProvider,
    chain_key: u64,
    latest_ethereum_block_number: u64,
) -> Result<CheckpointCreatedWithinRangeResult> {
    let checkpoint_interval = client
        .get_checkpoint_interval(chain_key)
        .await?
        .unwrap_or_default();
    info!("Checkpoint interval: {:?}", checkpoint_interval);
    let attestation_interval = client
        .get_attestation_interval(chain_key)
        .await?
        .unwrap_or_default();
    info!("Attestation interval: {:?}", attestation_interval);
    let attestation_vote_acceptance_window = client
        .get_attestation_vote_acceptance_window(chain_key)
        .await?
        .unwrap_or_default();
    info!(
        "Attestation vote acceptance window: {:?}",
        attestation_vote_acceptance_window
    );

    let checkpoint_block_range =
        checkpoint_interval as u64 * attestation_interval * attestation_vote_acceptance_window;

    info!("Checkpoint expected range: {}", checkpoint_block_range);

    let last_checkpoint = client
        .get_last_attestation_checkpoint(chain_key)
        .await?
        .context("No last checkpoint found")?;

    info!("Last checkpoint: {:?}", last_checkpoint);
    info!(
        "Latest Ethereum block number: {}",
        latest_ethereum_block_number
    );

    let mut checkpoint_created_within_range_checker = CheckpointCreatedWithinRangeResult {
        last_checkpoint_block_number: last_checkpoint.block_number,
        latest_ethereum_block_number,
        checkpoint_created_within_range: false,
    };

    if latest_ethereum_block_number.saturating_sub(last_checkpoint.block_number)
        <= checkpoint_block_range + MAX_ALLOWED_BLOCK_HEIGHT_DIFF as u64
    {
        checkpoint_created_within_range_checker.checkpoint_created_within_range = true;
        return Ok(checkpoint_created_within_range_checker);
    }

    Ok(checkpoint_created_within_range_checker)
}

pub fn calculate_usc_and_source_chain_block_diff(
    usc_block_height: u64,
    source_chain_block_height: u64,
) -> i128 {
    source_chain_block_height as i128 - usc_block_height as i128
}

pub(crate) async fn calculate_merkle_root(
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
