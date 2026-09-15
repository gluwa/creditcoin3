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

/// Default per-chain capacity (512 blocks) of the in-process raw block cache.
///
/// Each cached entry holds a whole source block's **decoded** transactions and receipts, so
/// this is the single largest per-chain allocation on a high-tx chain. Override it per chain
/// via `cache.block_cache_capacity`.
pub const DEFAULT_BLOCK_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(512) {
    Some(n) => n,
    None => panic!("512 is non-zero"),
};

/// Per-chain cache sizing, resolved from the optional `cache:` block of a chain entry.
///
/// Every field defaults to the historical behavior, so omitting `cache:` entirely changes
/// nothing. These exist because cache memory used to be a function purely of the chain's
/// on-chain attestation cadence and its transaction density -- neither of which this process
/// controls -- which let a high-throughput chain exhaust the whole process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainCacheConfig {
    /// Merkle-proof cache retention window, in source blocks.
    ///
    /// `None` derives it as `attestation_interval * checkpoint_interval * 4` from the chain's
    /// live on-chain intervals. That coupling is the catch: raising `attestation_interval` to
    /// *reduce* attestation load multiplies this window by the same factor. Pin it here to
    /// size the cache independently of attestation cadence.
    pub merkle_retention_blocks: Option<u64>,
    /// Soft byte budget for the merkle-proof cache.
    ///
    /// `None` means unbudgeted. When set, the effective retention window is narrowed using the
    /// cache's own measured bytes-per-block so the cache stays within budget regardless of how
    /// many transactions the chain puts in a block. Eviction itself stays height-ordered.
    pub merkle_max_bytes: Option<u64>,
    /// Capacity of the raw (decoded) block cache, in blocks. Defaults to
    /// [`DEFAULT_BLOCK_CACHE_CAPACITY`].
    pub block_cache_capacity: NonZeroUsize,
    /// Whether the background worker proactively fills the retention window.
    ///
    /// `true` (default) keeps the window warm at all times, which also means memory sits at
    /// its ceiling continuously. `false` fills the cache only when a proof actually needs a
    /// block, trading first-request latency for a much smaller resident set.
    pub merkle_backfill_enabled: bool,
    /// Cap on retained checkpoint-digest entries.
    ///
    /// `None` (default) retains every checkpoint, which is what proof serving needs: a proof
    /// is bracketed by the checkpoint immediately *below* the queried block, and there is no
    /// fallback to chain state on a miss. Setting this therefore caps how far back proofs can
    /// be served -- roughly `max_entries * attestation_interval * checkpoint_interval` blocks
    /// -- in exchange for bounding a cache that otherwise only ever grows. Opt in knowingly.
    pub checkpoint_cache_max_entries: Option<usize>,
}

impl Default for ChainCacheConfig {
    fn default() -> Self {
        Self {
            merkle_retention_blocks: None,
            merkle_max_bytes: None,
            block_cache_capacity: DEFAULT_BLOCK_CACHE_CAPACITY,
            merkle_backfill_enabled: true,
            checkpoint_cache_max_entries: None,
        }
    }
}

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
    /// Reorg-protection depth override, in blocks.
    ///
    /// `None` (the default when the field is omitted) means: derive it at startup from the
    /// chain's on-chain `MaturityStrategy` in the supported-chains pallet -- the same value the
    /// attestors use, so this process cannot disagree with them. `Some(n)` pins an explicit
    /// value; startup logs a WARN if it differs from the on-chain depth, and refuses to start if
    /// the chain follows a block tag (`RpcSafe` / `RpcFinalized`), which no fixed depth can
    /// reproduce.
    ///
    /// This used to be a plain `u64` defaulting to `0`, so *omitting* it silently disabled reorg
    /// protection. That is the failure mode this change removes.
    /// See [`continuity::ContinuityConfig::block_confirmation_depth`].
    pub block_confirmation_depth: Option<u64>,
    /// Per-chain cache sizing. Defaults reproduce the historical behavior.
    pub cache: ChainCacheConfig,
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
                // Mock config has no chain to resolve against; pin explicitly.
                block_confirmation_depth: Some(0),
                cache: ChainCacheConfig::default(),
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
    /// Reorg-protection depth override. **Omit it** to derive the depth from the chain's on-chain
    /// `MaturityStrategy` (recommended -- matches the attestors by construction). Set it only to
    /// deliberately pin a value; startup warns if it disagrees with the chain and fails if the
    /// chain follows a block tag (`RpcSafe` / `RpcFinalized`).
    #[serde(default)]
    pub block_confirmation_depth: Option<u64>,
    /// Optional per-chain cache sizing. Omit the whole block to keep the defaults.
    ///
    /// ```yaml
    /// cache:
    ///   merkle_retention_blocks: 1000
    ///   merkle_max_bytes: 805306368
    ///   block_cache_capacity: 96
    /// ```
    #[serde(default)]
    pub cache: ChainCacheConfigFile,
}

/// YAML layout of a chain's `cache:` block. Every field is optional; see
/// [`ChainCacheConfig`] for what each one means and what omitting it does.
#[derive(Debug, Default, Deserialize)]
pub struct ChainCacheConfigFile {
    #[serde(default)]
    pub merkle_retention_blocks: Option<u64>,
    /// Accepts a plain byte count or a human-readable size (`"768MiB"`, `"1GiB"`, `"800MB"`).
    #[serde(default, deserialize_with = "deserialize_byte_size")]
    pub merkle_max_bytes: Option<u64>,
    #[serde(default)]
    pub block_cache_capacity: Option<NonZeroUsize>,
    #[serde(default)]
    pub merkle_backfill_enabled: Option<bool>,
    #[serde(default)]
    pub checkpoint_cache_max_entries: Option<usize>,
}

/// Deserialize a byte size written either as a number or as a human-readable string.
///
/// A raw byte count for something like 768 MiB is 805306368, which is easy to fat-finger by a
/// factor of 1024 and impossible to eyeball in a review. Accepting `"768MiB"` keeps the
/// operator-facing value legible while still allowing a plain integer.
fn deserialize_byte_size<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Int(u64),
        Str(String),
    }

    match Option::<Raw>::deserialize(deserializer)? {
        None => Ok(None),
        Some(Raw::Int(bytes)) => Ok(Some(bytes)),
        Some(Raw::Str(text)) => parse_byte_size(&text)
            .map(Some)
            .map_err(serde::de::Error::custom),
    }
}

/// Parse `"768MiB"` / `"800MB"` / `"1024"` into a byte count.
///
/// Binary units (`KiB`/`MiB`/`GiB`) are powers of 1024; decimal units (`KB`/`MB`/`GB`) are
/// powers of 1000, matching how memory limits are usually quoted. Case-insensitive.
fn parse_byte_size(text: &str) -> std::result::Result<u64, String> {
    let trimmed = text.trim();
    let split = trimmed
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(trimmed.len());
    let (digits, unit) = trimmed.split_at(split);

    if digits.is_empty() {
        return Err(format!(
            "invalid byte size {text:?}: expected a number optionally followed by \
             B/KiB/MiB/GiB/KB/MB/GB"
        ));
    }

    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1_u64,
        "kib" => 1 << 10,
        "mib" => 1 << 20,
        "gib" => 1 << 30,
        "kb" => 1_000,
        "mb" => 1_000_000,
        "gb" => 1_000_000_000,
        other => {
            return Err(format!(
                "invalid byte size unit {other:?} in {text:?}: expected one of \
                 B, KiB, MiB, GiB, KB, MB, GB"
            ))
        }
    };

    digits
        .parse::<u64>()
        .map_err(|err| format!("invalid byte size {text:?}: {err}"))?
        .checked_mul(multiplier)
        .ok_or_else(|| format!("byte size {text:?} overflows u64"))
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
            let cache = resolve_cache_config(c.chain_key, c.cache)?;
            chains.push(ChainConfig {
                chain_key: c.chain_key,
                eth_rpc_url: c.eth_rpc_url,
                eth_rpc_fallback_urls,
                archiver_url: c.archiver_url,
                block_confirmation_depth: c.block_confirmation_depth,
                cache,
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

/// Resolve a chain's `cache:` block, filling omitted fields with their defaults.
///
/// # Errors
///
/// A zero for any of the sizing knobs. Zero is never a meaningful value here and it reads as
/// "unlimited" to the unwary, when it would in fact mean "cache nothing" -- so reject it and
/// point at the field that actually expresses the intent.
///
/// `chain_key` is included in error messages to help users locate the offending entry in a
/// multi-chain config.
fn resolve_cache_config(chain_key: u64, file: ChainCacheConfigFile) -> Result<ChainCacheConfig> {
    if file.merkle_retention_blocks == Some(0) {
        bail!(
            "chain_key {chain_key}: `cache.merkle_retention_blocks` must be greater than 0; \
             omit it to derive the window from the chain's attestation intervals"
        );
    }
    if file.merkle_max_bytes == Some(0) {
        bail!(
            "chain_key {chain_key}: `cache.merkle_max_bytes` must be greater than 0; \
             omit it for an unbudgeted cache"
        );
    }
    if file.checkpoint_cache_max_entries == Some(0) {
        bail!(
            "chain_key {chain_key}: `cache.checkpoint_cache_max_entries` must be greater than 0; \
             omit it to retain every checkpoint"
        );
    }

    let defaults = ChainCacheConfig::default();
    Ok(ChainCacheConfig {
        merkle_retention_blocks: file.merkle_retention_blocks,
        merkle_max_bytes: file.merkle_max_bytes,
        block_cache_capacity: file
            .block_cache_capacity
            .unwrap_or(defaults.block_cache_capacity),
        merkle_backfill_enabled: file
            .merkle_backfill_enabled
            .unwrap_or(defaults.merkle_backfill_enabled),
        checkpoint_cache_max_entries: file.checkpoint_cache_max_entries,
    })
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
    fn yaml_without_depth_is_none_not_zero() {
        // Regression guard: omitting the field must mean "derive from chain", never "0".
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert_eq!(cfg.chains[0].block_confirmation_depth, None);
    }

    #[test]
    fn yaml_with_explicit_depth_is_some() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    block_confirmation_depth: 64
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert_eq!(cfg.chains[0].block_confirmation_depth, Some(64));
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

    #[test]
    fn yaml_without_cache_block_uses_defaults() {
        // Regression guard: omitting `cache:` must reproduce the historical behavior exactly,
        // i.e. derive the retention window and leave the caches unbudgeted.
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
"#;
        let cfg = parse(yaml).expect("yaml should parse");

        assert_eq!(cfg.chains[0].cache, ChainCacheConfig::default());
        assert_eq!(cfg.chains[0].cache.merkle_retention_blocks, None);
        assert_eq!(cfg.chains[0].cache.merkle_max_bytes, None);
        assert_eq!(cfg.chains[0].cache.checkpoint_cache_max_entries, None);
        assert!(cfg.chains[0].cache.merkle_backfill_enabled);
        assert_eq!(
            cfg.chains[0].cache.block_cache_capacity,
            DEFAULT_BLOCK_CACHE_CAPACITY
        );
    }

    #[test]
    fn yaml_with_cache_block_is_parsed() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      merkle_retention_blocks: 1000
      merkle_max_bytes: 805306368
      block_cache_capacity: 96
      merkle_backfill_enabled: false
      checkpoint_cache_max_entries: 20000
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        let cache = &cfg.chains[0].cache;

        assert_eq!(cache.merkle_retention_blocks, Some(1000));
        assert_eq!(cache.merkle_max_bytes, Some(805_306_368));
        assert_eq!(cache.block_cache_capacity.get(), 96);
        assert!(!cache.merkle_backfill_enabled);
        assert_eq!(cache.checkpoint_cache_max_entries, Some(20_000));
    }

    #[test]
    fn partial_cache_block_keeps_other_defaults() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      merkle_retention_blocks: 1000
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        let cache = &cfg.chains[0].cache;

        assert_eq!(cache.merkle_retention_blocks, Some(1000));
        assert_eq!(cache.block_cache_capacity, DEFAULT_BLOCK_CACHE_CAPACITY);
        assert!(cache.merkle_backfill_enabled);
    }

    #[test]
    fn cache_block_rejects_zero_sizes() {
        for (field, value) in [
            ("merkle_retention_blocks", "0"),
            ("merkle_max_bytes", "0"),
            ("checkpoint_cache_max_entries", "0"),
        ] {
            let yaml = format!(
                r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      {field}: {value}
"#
            );
            let err = parse(&yaml).expect_err("zero should be rejected");
            let msg = err.to_string();
            assert!(
                msg.contains(field) && msg.contains("chain_key 8"),
                "wrong error: {msg}"
            );
        }
    }

    #[test]
    fn cache_block_rejects_zero_block_cache_capacity() {
        // NonZeroUsize makes serde itself reject this one.
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      block_cache_capacity: 0
"#;
        assert!(parse(yaml).is_err(), "zero capacity should be rejected");
    }

    #[test]
    fn byte_sizes_accept_units_and_plain_numbers() {
        for (text, expected) in [
            ("1024", 1024_u64),
            ("512B", 512),
            ("768MiB", 768 * 1024 * 1024),
            ("1GiB", 1024 * 1024 * 1024),
            ("4KiB", 4096),
            ("800MB", 800_000_000),
            ("2gb", 2_000_000_000),
            (" 768 MiB ", 768 * 1024 * 1024),
        ] {
            assert_eq!(
                parse_byte_size(text),
                Ok(expected),
                "parsing {text:?} should give {expected}"
            );
        }
    }

    #[test]
    fn byte_sizes_reject_nonsense() {
        for text in ["", "MiB", "768 mib mib", "12 furlongs", "-5"] {
            assert!(
                parse_byte_size(text).is_err(),
                "{text:?} should be rejected"
            );
        }
    }

    #[test]
    fn merkle_max_bytes_accepts_a_readable_unit_in_yaml() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      merkle_max_bytes: "768MiB"
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert_eq!(
            cfg.chains[0].cache.merkle_max_bytes,
            Some(768 * 1024 * 1024)
        );
    }

    #[test]
    fn merkle_max_bytes_still_accepts_a_plain_integer() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      merkle_max_bytes: 805306368
"#;
        let cfg = parse(yaml).expect("yaml should parse");
        assert_eq!(cfg.chains[0].cache.merkle_max_bytes, Some(805_306_368));
    }

    #[test]
    fn merkle_max_bytes_rejects_a_bad_unit() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
    cache:
      merkle_max_bytes: "768 gigglebytes"
"#;
        let err = parse(yaml).expect_err("bad unit should be rejected");
        let msg = err.to_string();
        assert!(msg.contains("invalid byte size unit"), "wrong error: {msg}");
    }

    #[test]
    fn shipped_example_config_parses() {
        // The example file is the documentation for these knobs; keep it loadable so a stray
        // edit to the commented blocks cannot ship broken YAML.
        let yaml = include_str!("../config.example.yaml");
        let cfg = parse(yaml).expect("config.example.yaml should parse");

        assert!(!cfg.chains.is_empty());
        // Everything under `cache:` is commented out there, so defaults must survive.
        for chain in &cfg.chains {
            assert_eq!(chain.cache, ChainCacheConfig::default());
        }
    }

    #[test]
    fn duplicate_chain_key_is_rejected() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains:
  - chain_key: 8
    eth_rpc_url: "http://localhost:8545"
  - chain_key: 8
    eth_rpc_url: "http://localhost:8546"
"#;
        let err = parse(yaml).expect_err("duplicate chain_key should be rejected");
        let msg = err.to_string();
        assert!(msg.contains("duplicate chain_key 8"), "wrong error: {msg}");
    }

    #[test]
    fn empty_chains_list_is_rejected() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3100
chains: []
"#;
        let err = parse(yaml).expect_err("empty chains should be rejected");
        let msg = err.to_string();
        assert!(msg.contains("at least one entry"), "wrong error: {msg}");
    }
}
