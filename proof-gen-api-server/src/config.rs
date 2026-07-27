use anyhow::{bail, Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::num::NonZeroUsize;
use std::path::Path;

/// Default `max_batch_size` (10) for CLI, YAML, and [`Config::new_mock_config`].
pub const DEFAULT_MAX_BATCH_SIZE: NonZeroUsize = match NonZeroUsize::new(10) {
    Some(n) => n,
    None => panic!("10 is non-zero"),
};

/// Default `max_batch_span` (1 000 blocks) - maximum distance between the
/// lowest and highest block in a single batch request. Prevents a small batch
/// from forcing proof generation over an extremely large block range.
pub const DEFAULT_MAX_BATCH_SPAN: u64 = 1_000;

/// One source chain (EVM) served by this process, keyed on Creditcoin3.
#[derive(Debug, Clone)]
pub struct ChainConfig {
    pub chain_key: u64,
    /// Primary RPC URL: tried first for every operation, and the only URL
    /// used for tip-related calls (subscription, current block height).
    pub eth_rpc_url: String,
    /// Ordered fallback RPC URLs. The [`eth::Client`] tries the primary
    /// first and walks this list in declaration order whenever the primary
    /// returns `Ok(None)` or a transport error for a block fetch / tx-hash
    /// lookup. Useful when the primary is a cheap "recent-only" endpoint
    /// and you keep a more expensive "archive" endpoint for old data.
    pub eth_rpc_fallback_urls: Vec<String>,
    pub archiver_url: Option<String>,
    /// Number of blocks to lag behind the EVM chain tip for reorg protection.
    /// See [`continuity::ContinuityConfig::block_confirmation_depth`].
    pub block_confirmation_depth: u64,
}

/// Server configuration after CLI / file resolution.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    pub cc3_rpc_url: String,
    pub cc3_key: Option<String>,
    pub chains: Vec<ChainConfig>,
    pub max_batch_size: NonZeroUsize,
    pub max_batch_span: u64,
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
                eth_rpc_fallback_urls: Vec::new(),
                archiver_url: None,
                block_confirmation_depth: 0,
            }],
            max_batch_size: DEFAULT_MAX_BATCH_SIZE,
            max_batch_span: DEFAULT_MAX_BATCH_SPAN,
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
    /// Deprecated – accepted for backward compat but ignored at runtime.
    #[serde(default)]
    pub indexer_url: Option<String>,
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: NonZeroUsize,
    #[serde(default = "default_max_batch_span")]
    pub max_batch_span: u64,
}

#[derive(Debug, Deserialize)]
pub struct ChainConfigFile {
    pub chain_key: u64,
    pub eth_rpc_url: String,
    /// Optional ordered list of fallback RPC URLs. The first non-empty
    /// answer wins; the primary `eth_rpc_url` is always tried first.
    ///
    /// ```yaml
    /// eth_rpc_url: "https://recent.example/v2/<KEY_RECENT>"
    /// eth_rpc_fallback_urls:
    ///   - "https://archive.example/v2/<KEY_ARCHIVE>"
    /// ```
    #[serde(default)]
    pub eth_rpc_fallback_urls: Vec<String>,
    #[serde(default)]
    pub archiver_url: Option<String>,
    /// Number of blocks to lag behind the EVM chain tip for reorg protection.
    /// Defaults to 0 (no lag) for backward compatibility.
    #[serde(default)]
    pub block_confirmation_depth: u64,
}

fn default_max_batch_size() -> NonZeroUsize {
    DEFAULT_MAX_BATCH_SIZE
}

fn default_max_batch_span() -> u64 {
    DEFAULT_MAX_BATCH_SPAN
}

impl ConfigFile {
    fn into_config(self, cc3_rpc_url: String) -> Result<Config> {
        if self.chains.is_empty() {
            bail!("config must include at least one entry in `chains`");
        }
        let mut seen = HashSet::new();
        let mut chains = Vec::with_capacity(self.chains.len());
        for c in self.chains {
            if !seen.insert(c.chain_key) {
                bail!("duplicate chain_key {} in config", c.chain_key);
            }
            let eth_rpc_fallback_urls =
                validate_fallback_urls(c.chain_key, c.eth_rpc_fallback_urls)?;
            chains.push(ChainConfig {
                chain_key: c.chain_key,
                eth_rpc_url: c.eth_rpc_url,
                eth_rpc_fallback_urls,
                archiver_url: c.archiver_url,
                block_confirmation_depth: c.block_confirmation_depth,
            });
        }
        Ok(Config {
            bind_host: self.bind_host,
            bind_port: self.bind_port,
            cc3_rpc_url,
            cc3_key: self.cc3_key,
            chains,
            max_batch_size: self.max_batch_size,
            max_batch_span: self.max_batch_span,
        })
    }
}

/// Validate a chain's `eth_rpc_fallback_urls` (purely structural — no network
/// I/O). Returns the trimmed list, preserving declaration order.
///
/// # Errors
///
/// * Any URL is empty / whitespace-only.
/// * The same URL appears twice in one chain's fallback list (likely
///   misconfiguration — duplicates would just hit the same endpoint twice).
///
/// `chain_key` is included in error messages to help users locate the
/// offending entry in a multi-chain config.
fn validate_fallback_urls(chain_key: u64, urls: Vec<String>) -> Result<Vec<String>> {
    if urls.is_empty() {
        return Ok(Vec::new());
    }

    let mut trimmed: Vec<String> = Vec::with_capacity(urls.len());
    for (idx, raw) in urls.into_iter().enumerate() {
        let url = raw.trim().to_string();
        if url.is_empty() {
            bail!(
                "chain_key {chain_key}: `eth_rpc_fallback_urls[{idx}]` is empty; \
                 remove the entry or set a real URL"
            );
        }
        if trimmed.iter().any(|existing| existing == &url) {
            bail!(
                "chain_key {chain_key}: duplicate URL in `eth_rpc_fallback_urls` (index {idx}); \
                 each fallback must be a distinct endpoint"
            );
        }
        trimmed.push(url);
    }

    Ok(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Config> {
        let file: ConfigFile = serde_yaml::from_str(yaml)?;
        file.into_config("ws://test".to_string())
    }

    #[test]
    fn validate_fallbacks_accepts_empty_list() {
        let out = validate_fallback_urls(2, vec![]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn validate_fallbacks_rejects_empty_url() {
        let err = validate_fallback_urls(2, vec!["   ".to_string()]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("chain_key 2"), "missing chain_key: {msg}");
        assert!(
            msg.contains("`eth_rpc_fallback_urls[0]` is empty"),
            "wrong error: {msg}"
        );
    }

    #[test]
    fn validate_fallbacks_rejects_duplicate_url() {
        let err = validate_fallback_urls(
            7,
            vec![
                "https://archive.example/v2/KEY_A".to_string(),
                "https://archive.example/v2/KEY_A".to_string(),
            ],
        )
        .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("chain_key 7"), "missing chain_key: {msg}");
        assert!(
            msg.contains("duplicate URL in `eth_rpc_fallback_urls`"),
            "wrong error: {msg}"
        );
    }

    #[test]
    fn validate_fallbacks_trims_whitespace() {
        // Operators sometimes have stray whitespace from copy/paste; trim it
        // rather than failing with an opaque DNS error later.
        let out =
            validate_fallback_urls(2, vec!["  https://archive.example/v2/KEY_A  ".to_string()])
                .unwrap();
        assert_eq!(out, vec!["https://archive.example/v2/KEY_A".to_string()]);
    }

    #[test]
    fn validate_fallbacks_keeps_declaration_order() {
        let out = validate_fallback_urls(2, vec!["http://b".to_string(), "http://a".to_string()])
            .unwrap();
        assert_eq!(out, vec!["http://b".to_string(), "http://a".to_string()]);
    }

    #[test]
    fn yaml_round_trip_with_fallback_urls() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 2
    eth_rpc_url: "https://recent.example/v2/KEY_RECENT"
    eth_rpc_fallback_urls:
      - "https://archive.example/v2/KEY_ARCHIVE"
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert_eq!(cfg.chains.len(), 1);
        let chain = &cfg.chains[0];
        assert_eq!(chain.eth_rpc_fallback_urls.len(), 1);
        assert_eq!(
            chain.eth_rpc_fallback_urls[0],
            "https://archive.example/v2/KEY_ARCHIVE"
        );
    }

    #[test]
    fn yaml_without_fallbacks_keeps_field_empty() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 2
    eth_rpc_url: "http://localhost:8545"
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert!(cfg.chains[0].eth_rpc_fallback_urls.is_empty());
    }

    #[test]
    fn yaml_with_empty_fallback_url_fails_to_parse() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 2
    eth_rpc_url: "http://localhost:8545"
    eth_rpc_fallback_urls:
      - ""
"#;
        let err = parse(yaml).unwrap_err().to_string();
        assert!(
            err.contains("`eth_rpc_fallback_urls[0]` is empty"),
            "expected empty-url error, got: {err}"
        );
    }
}
