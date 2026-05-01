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
    /// Default RPC URL used for tip-related calls (subscription, current
    /// block height) and for any block-keyed call whose `block_number` is
    /// not covered by an entry in [`Self::eth_rpc_overrides`].
    pub eth_rpc_url: String,
    /// Per-block-range RPC URL overrides. When a block-keyed RPC call's
    /// `block_number` falls within an override's `[from_block, to_block]`
    /// range (both bounds inclusive; either may be `None` for an
    /// open-ended side), that override's URL is used instead of
    /// [`Self::eth_rpc_url`]. See [`RpcRangeOverride`].
    ///
    /// Useful when different block ranges live on different RPC providers
    /// (and therefore require different API keys), e.g. a cheap "recent"
    /// endpoint plus a more expensive "archive" endpoint for old blocks.
    pub eth_rpc_overrides: Vec<RpcRangeOverride>,
    pub archiver_url: Option<String>,
    /// Number of blocks to lag behind the EVM chain tip for reorg protection.
    /// See [`continuity::ContinuityConfig::block_confirmation_depth`].
    pub block_confirmation_depth: u64,
}

/// One per-block-range RPC URL override for a single chain.
///
/// This is the runtime mirror of [`RpcRangeOverrideFile`] (the YAML
/// representation). It also converts cleanly into [`eth::RpcRangeOverride`]
/// at the point where the [`eth::Client`] is constructed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcRangeOverride {
    /// Inclusive lower bound (in source-chain block numbers). `None` means
    /// "no lower bound" (i.e. matches blocks `0..=to_block`).
    pub from_block: Option<u64>,
    /// Inclusive upper bound. `None` means "no upper bound" (i.e. matches
    /// blocks `from_block..=u64::MAX`).
    pub to_block: Option<u64>,
    /// RPC endpoint used when this override matches. Typically embeds an
    /// API key in the URL path or query string.
    pub url: String,
}

impl From<&RpcRangeOverride> for eth::RpcRangeOverride {
    fn from(o: &RpcRangeOverride) -> Self {
        eth::RpcRangeOverride {
            from_block: o.from_block,
            to_block: o.to_block,
            url: o.url.clone(),
        }
    }
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
                eth_rpc_overrides: Vec::new(),
                archiver_url: None,
                block_confirmation_depth: 0,
            }],
            redis_url: None,
            redis_cluster_mode: false,
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
    #[serde(default)]
    pub redis_url: Option<String>,
    #[serde(default)]
    pub redis_cluster_mode: bool,
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
    /// Optional list of per-block-range RPC URL overrides. See
    /// [`RpcRangeOverrideFile`] for the YAML shape and validation rules.
    #[serde(default)]
    pub eth_rpc_overrides: Vec<RpcRangeOverrideFile>,
    #[serde(default)]
    pub archiver_url: Option<String>,
    /// Number of blocks to lag behind the EVM chain tip for reorg protection.
    /// Defaults to 0 (no lag) for backward compatibility.
    #[serde(default)]
    pub block_confirmation_depth: u64,
}

/// YAML representation of one entry in `eth_rpc_overrides`.
///
/// Either bound may be omitted to mean "open-ended on that side", but at
/// least one bound must be present (otherwise the override would silently
/// shadow `eth_rpc_url`). Both bounds are inclusive.
///
/// ```yaml
/// eth_rpc_overrides:
///   - to_block: 4000000
///     url: "https://archive.example/v2/<KEY_A>"
///   - from_block: 4000001
///     url: "https://recent.example/v2/<KEY_B>"
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct RpcRangeOverrideFile {
    #[serde(default)]
    pub from_block: Option<u64>,
    #[serde(default)]
    pub to_block: Option<u64>,
    pub url: String,
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
            let eth_rpc_overrides = validate_rpc_overrides(c.chain_key, c.eth_rpc_overrides)?;
            chains.push(ChainConfig {
                chain_key: c.chain_key,
                eth_rpc_url: c.eth_rpc_url,
                eth_rpc_overrides,
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
            redis_url: self.redis_url,
            redis_cluster_mode: self.redis_cluster_mode,
            max_batch_size: self.max_batch_size,
            max_batch_span: self.max_batch_span,
        })
    }
}

/// Validate a chain's `eth_rpc_overrides` (purely structural — no network
/// I/O). Returns the runtime [`RpcRangeOverride`] vector preserving the
/// declaration order; `eth::Client` will sort by `from_block` and re-validate
/// non-overlap when it actually opens connections.
///
/// # Errors
///
/// * Any override has neither `from_block` nor `to_block`
///   ("require_default" semantics — the bare `eth_rpc_url` is the implicit
///   default; an unbounded override would silently shadow it).
/// * Any override has `from_block > to_block`.
/// * Two overrides cover an overlapping block range (touching ranges with
///   a shared boundary count as overlapping, since both endpoints would
///   match a block at the boundary).
///
/// `chain_key` is included in error messages to help users locate the
/// offending entry in a multi-chain config.
fn validate_rpc_overrides(
    chain_key: u64,
    overrides: Vec<RpcRangeOverrideFile>,
) -> Result<Vec<RpcRangeOverride>> {
    if overrides.is_empty() {
        return Ok(Vec::new());
    }

    // Pre-flight bound checks.
    let mut bounds: Vec<(u64, u64, RpcRangeOverride)> = Vec::with_capacity(overrides.len());
    for ov in overrides {
        if ov.from_block.is_none() && ov.to_block.is_none() {
            bail!(
                "chain_key {chain_key}: every entry in `eth_rpc_overrides` must set at least \
                 one of `from_block` or `to_block` (an unbounded override would silently \
                 shadow `eth_rpc_url`); use `eth_rpc_url` directly for the default URL"
            );
        }
        let from = ov.from_block.unwrap_or(0);
        let to = ov.to_block.unwrap_or(u64::MAX);
        if from > to {
            bail!(
                "chain_key {chain_key}: `eth_rpc_overrides` entry has from_block ({from}) \
                 > to_block ({to}); swap the bounds in your config"
            );
        }
        bounds.push((
            from,
            to,
            RpcRangeOverride {
                from_block: ov.from_block,
                to_block: ov.to_block,
                url: ov.url,
            },
        ));
    }

    // Overlap check (sort by `from`; sliding window).
    let mut sorted = bounds.clone();
    sorted.sort_by_key(|(from, _, _)| *from);
    for w in sorted.windows(2) {
        let (a_from, a_to, _) = &w[0];
        let (b_from, b_to, _) = &w[1];
        if a_to >= b_from {
            bail!(
                "chain_key {chain_key}: `eth_rpc_overrides` entries overlap — \
                 [{a_from}, {a_to}] overlaps [{b_from}, {b_to}]"
            );
        }
    }

    Ok(bounds.into_iter().map(|(_, _, ov)| ov).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ov_file(from: Option<u64>, to: Option<u64>, url: &str) -> RpcRangeOverrideFile {
        RpcRangeOverrideFile {
            from_block: from,
            to_block: to,
            url: url.to_string(),
        }
    }

    fn parse(yaml: &str) -> Result<Config> {
        let file: ConfigFile = serde_yaml::from_str(yaml)?;
        file.into_config("ws://test".to_string())
    }

    #[test]
    fn validate_overrides_accepts_empty_list() {
        let out = validate_rpc_overrides(2, vec![]).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn validate_overrides_rejects_unbounded_entry() {
        let err = validate_rpc_overrides(2, vec![ov_file(None, None, "http://x")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("chain_key 2"), "missing chain_key: {msg}");
        assert!(
            msg.contains("at least one of `from_block` or `to_block`"),
            "wrong error: {msg}"
        );
    }

    #[test]
    fn validate_overrides_rejects_inverted_bounds() {
        let err =
            validate_rpc_overrides(7, vec![ov_file(Some(100), Some(50), "http://x")]).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("chain_key 7"), "missing chain_key: {msg}");
        assert!(
            msg.contains("from_block (100) > to_block (50)"),
            "wrong error: {msg}"
        );
    }

    #[test]
    fn validate_overrides_rejects_overlap() {
        let err = validate_rpc_overrides(
            3,
            vec![
                ov_file(Some(0), Some(1_000), "http://low"),
                ov_file(Some(500), Some(2_000), "http://mid"),
            ],
        )
        .unwrap_err();
        assert!(err.to_string().contains("overlap"), "wrong error: {err}");
    }

    #[test]
    fn validate_overrides_treats_touching_ranges_as_overlap() {
        // Both ends inclusive — block 100 would match both ranges.
        let err = validate_rpc_overrides(
            3,
            vec![
                ov_file(Some(0), Some(100), "http://low"),
                ov_file(Some(100), Some(200), "http://hi"),
            ],
        )
        .unwrap_err();
        assert!(err.to_string().contains("overlap"));
    }

    #[test]
    fn validate_overrides_keeps_declaration_order() {
        // Declaration order is preserved (downstream `eth::Client` will sort
        // by `from_block` when wiring real connections).
        let out = validate_rpc_overrides(
            2,
            vec![
                ov_file(Some(4_000_001), None, "http://recent"),
                ov_file(None, Some(4_000_000), "http://archive"),
            ],
        )
        .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].url, "http://recent");
        assert_eq!(out[1].url, "http://archive");
    }

    #[test]
    fn yaml_round_trip_with_two_bucket_overrides() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 2
    eth_rpc_url: "https://default.example/v2/KEY_DEFAULT"
    eth_rpc_overrides:
      - to_block: 4000000
        url: "https://archive.example/v2/KEY_A"
      - from_block: 4000001
        url: "https://recent.example/v2/KEY_B"
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert_eq!(cfg.chains.len(), 1);
        let chain = &cfg.chains[0];
        assert_eq!(chain.eth_rpc_overrides.len(), 2);
        assert_eq!(chain.eth_rpc_overrides[0].to_block, Some(4_000_000));
        assert_eq!(chain.eth_rpc_overrides[1].from_block, Some(4_000_001));
    }

    #[test]
    fn yaml_without_overrides_keeps_field_empty() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 2
    eth_rpc_url: "http://localhost:8545"
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert!(cfg.chains[0].eth_rpc_overrides.is_empty());
    }

    #[test]
    fn yaml_with_invalid_overrides_fails_to_parse() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 2
    eth_rpc_url: "http://localhost:8545"
    eth_rpc_overrides:
      - url: "http://oops-no-bounds"
"#;
        let err = parse(yaml).unwrap_err().to_string();
        assert!(
            err.contains("at least one of `from_block` or `to_block`"),
            "expected unbounded-override error, got: {err}"
        );
    }
}
