use clap::Parser;
use futures::future::BoxFuture;
use serde::Deserialize;
use std::path::PathBuf;

mod attestation_check_result;
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

pub type BoxFutureResult<'a, T, E = anyhow::Error> = BoxFuture<'a, Result<Option<T>, E>>;

pub fn calculate_usc_and_source_chain_block_diff(
    usc_block_height: u64,
    source_chain_block_height: u64,
) -> i128 {
    source_chain_block_height as i128 - usc_block_height as i128
}
