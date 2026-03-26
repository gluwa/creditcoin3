use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// One source chain (EVM) served by this process, keyed on Creditcoin3.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub chain_key: u64,
    pub eth_rpc_url: String,
    pub archiver_url: Option<String>,
}

/// Server configuration after CLI / file resolution.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    pub cc3_rpc_url: String,
    pub cc3_key: Option<String>,
    pub chains: Vec<ChainConfig>,
    pub redis_url: Option<String>,
    pub redis_cluster_mode: bool,
    pub indexer_url: Option<String>,
    pub max_batch_size: usize,
}

impl Config {
    /// Convenience constructor for tests — one chain, dummy endpoints.
    pub fn new_mock_config(chain_key: u64) -> Self {
        Self {
            bind_host: "127.0.0.1".to_string(),
            bind_port: 3000,
            cc3_rpc_url: "ws://mock".to_string(),
            cc3_key: None,
            chains: vec![ChainConfig {
                chain_key,
                eth_rpc_url: "http://mock".to_string(),
                archiver_url: None,
            }],
            redis_url: None,
            redis_cluster_mode: false,
            indexer_url: None,
            max_batch_size: 10,
        }
    }

    pub fn chain_keys(&self) -> HashSet<u64> {
        self.chains.iter().map(|c| c.chain_key).collect()
    }

    /// Load YAML configuration from disk (see `.env.example` / `config.example.yaml`).
    /// Creditcoin3 WebSocket URL is not stored in YAML; pass `cc3_rpc_url` from `CC3_RPC_URL` / CLI.
    pub fn from_yaml_file(path: impl AsRef<Path>, cc3_rpc_url: String) -> Result<Self> {
        let text = fs::read_to_string(path.as_ref()).with_context(|| {
            format!(
                "Failed to read proof-gen config file {}",
                path.as_ref().display()
            )
        })?;
        let file: ConfigFile = serde_yaml::from_str(&text).context("Invalid YAML config")?;
        file.into_config(cc3_rpc_url)
    }
}

/// YAML file layout (shared fields + `chains` list).
#[derive(Debug, Deserialize)]
pub struct ConfigFile {
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub cc3_key: Option<String>,
    pub chains: Vec<ChainConfigFile>,
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default)]
    pub redis_cluster_mode: bool,
    #[serde(default)]
    pub indexer_url: Option<String>,
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
}

#[derive(Debug, Deserialize)]
pub struct ChainConfigFile {
    pub chain_key: u64,
    pub eth_rpc_url: String,
    #[serde(default)]
    pub archiver_url: Option<String>,
}

fn default_max_batch_size() -> usize {
    10
}

impl ConfigFile {
    fn into_config(self, cc3_rpc_url: String) -> Result<Config> {
        if self.chains.is_empty() {
            bail!("config must include at least one entry in `chains`");
        }
        if self.max_batch_size == 0 {
            bail!("max_batch_size must be greater than 0");
        }
        let mut seen = HashSet::new();
        let mut chains = Vec::with_capacity(self.chains.len());
        for c in self.chains {
            if !seen.insert(c.chain_key) {
                bail!("duplicate chain_key {} in config", c.chain_key);
            }
            chains.push(ChainConfig {
                chain_key: c.chain_key,
                eth_rpc_url: c.eth_rpc_url,
                archiver_url: c.archiver_url,
            });
        }
        Ok(Config {
            bind_host: self.bind_host,
            bind_port: self.bind_port,
            cc3_rpc_url,
            cc3_key: self.cc3_key,
            chains,
            redis_url: self.redis_url,
            redis_cluster_mode: self.redis_cluster_mode,
            indexer_url: self.indexer_url,
            max_batch_size: self.max_batch_size,
        })
    }
}
