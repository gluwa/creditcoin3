use anyhow::{Context, Result};
use attestor_primitives::{AttestationCheckpoint, SignedAttestation};
use cc_client::Client as USCClient;
use clap::Parser;
use eth::{self, AlloyB256, Client as EthClient, OrderedBlock};
use ethers::types::U64;
use mmr::traits::MerkleTreeTrait;
use mockall::{automock, predicate::*};
use serde::Deserialize;
use sp_core::H256;
use std::path::PathBuf;
use subxt::utils::AccountId32;
use tracing::info;
const MAX_ALLOWED_BLOCK_HEIGHT_DIFF: i128 = 50;

pub mod attestation_check_result;
pub mod attestation_checks;
mod create_json_message;

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

/// - `usc_network_name`: Name of the USC network (for logging purposes)
/// - `usc_rpc_url`: USC RPC url (must have rpc + websocket features)
/// - `usc_account_mnemonic`: Mnemonic for a usc account
/// - `ethereum_rpc_url`: Ethereum RPC url (for fetching block height)
/// - `chain_key`: Chain key of the source chain to monitor
#[derive(Debug, Deserialize, Clone)]
pub struct NetworkTarget {
    pub usc_network_name: String,
    pub usc_rpc_url: String,
    pub usc_account_mnemonic: String,
    pub ethereum_rpc_url: String,
    pub chain_key: u64,
}

#[derive(Debug, Deserialize, Default)]
pub struct SanitiesConfigFile {
    #[serde(default)]
    pub targets: Vec<NetworkTarget>,
    #[serde(default)]
    pub slack_webhook_url: String,
    #[serde(default)]
    pub slack_alert_group: Option<String>,
    #[serde(default)]
    pub log_verbose: bool,
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
    let vote_acceptance_window = client
        .get_attestation_vote_acceptance_window(chain_key)
        .await?
        .unwrap_or_default();
    info!("Vote acceptance window: {:?}", vote_acceptance_window);

    // Number of attestations between checkpoints
    let checkpoint_block_range =
        checkpoint_interval as u64 * attestation_interval * vote_acceptance_window;

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
