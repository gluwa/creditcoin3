//! Runtime configuration for the message relayer.
//!
//! The shape mirrors `proof-gen-api-server/src/config.rs`: a [`ConfigFile`] that maps to YAML
//! verbatim and a [`Config`] that holds the validated, typed runtime values. CLI overrides are
//! applied in `bin/relayer.rs`.

use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use alloy::primitives::Address;
use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// Default vote cache TTL (seconds). Drops incomplete aggregates to bound RAM (PoC §9).
pub const DEFAULT_VOTE_CACHE_TTL_SECS: u64 = 30 * 60;
/// Default vote cache size cap (number of distinct messages tracked at once).
pub const DEFAULT_VOTE_CACHE_MAX_MESSAGES: usize = 10_000;
/// Default `simulate_before_send`; we always simulate in PoC to catch `validateVotes` reverts.
pub const DEFAULT_SIMULATE_BEFORE_SEND: bool = true;
/// Default delivery retry budget per message before giving up.
pub const DEFAULT_DELIVERY_MAX_RETRIES: u32 = 5;
/// Default gas multiplier (estimated gas * this) for `deliverMessage`.
pub const DEFAULT_GAS_MULTIPLIER: f64 = 1.2;
/// Default P2P TCP port.
pub const DEFAULT_P2P_PORT: u16 = 9100;
/// Default reorg lag (blocks) for source chain finality.
pub const DEFAULT_BLOCK_CONFIRMATION_DEPTH: u64 = 0;

// ---------------------------------------------------------------------------
// Validated runtime config
// ---------------------------------------------------------------------------

/// Validated runtime configuration assembled by [`Config::from_yaml_file`] or by the CLI
/// `--single-route` quickstart.
#[derive(Debug, Clone)]
pub struct Config {
    pub bind_host: String,
    pub bind_port: u16,
    /// Creditcoin Substrate RPC. Reserved for future on-chain attestor-set resolution; not
    /// strictly required when all routes use a static `attestor_set`.
    pub cc3_rpc_url: String,
    /// Creditcoin EVM RPC. The relayer reads `MessagePublished` events from the configured
    /// Outboxes and `eth_chainId` from this endpoint.
    pub creditcoin_eth_rpc_url: String,
    pub p2p: P2pConfig,
    pub vote_cache: VoteCacheConfig,
    pub delivery: DeliveryConfig,
    pub routes: Vec<ChainRoute>,
}

#[derive(Debug, Clone)]
pub struct ChainRoute {
    pub chain_key: u64,
    pub creditcoin_chain_id: u64,
    pub outbox_address: Option<Address>,
    pub destination_rpc_url: String,
    pub inbox_address: Address,
    pub signer_key: Option<String>,
    pub block_confirmation_depth: u64,
    pub attestor_set: AttestorSet,
    pub threshold_override: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum AttestorSet {
    /// Static EVM-address allowlist supplied in config. Used for the PoC.
    Static(Vec<Address>),
    /// On-chain `IVoteValidator` introspection (stub until validator contract is finalized).
    OnChain { source: AttestorSource },
}

#[derive(Debug, Clone)]
pub enum AttestorSource {
    /// Validator contract on the destination chain.
    Evm { address: Address },
    /// Active attestor set on the Creditcoin runtime for `chain_key`.
    Cc3 { chain_key: u64 },
}

#[derive(Debug, Clone)]
pub struct P2pConfig {
    pub port: u16,
    pub public_addr: Option<String>,
    pub boot_nodes: Vec<String>,
    pub no_mdns: bool,
    pub identity: Option<String>,
}

#[derive(Debug, Clone)]
pub struct VoteCacheConfig {
    pub ttl_seconds: u64,
    pub max_messages: usize,
}

#[derive(Debug, Clone)]
pub struct DeliveryConfig {
    pub simulate_before_send: bool,
    pub max_retries: u32,
    pub gas_multiplier: f64,
}

impl Config {
    /// Build a [`Config`] from a YAML file. The Creditcoin RPC URLs are supplied separately
    /// because they are commonly overridden from CLI/env (mirrors `proof-gen-api-server`).
    pub fn from_yaml_file(
        path: impl AsRef<Path>,
        cc3_rpc_url: String,
        creditcoin_eth_rpc_url: String,
    ) -> Result<Self> {
        let text = fs::read_to_string(path.as_ref()).with_context(|| {
            format!(
                "Failed to read message-relayer config file {}",
                path.as_ref().display()
            )
        })?;
        let file: ConfigFile = serde_yaml::from_str(&text).context("Invalid YAML config")?;
        file.into_config(cc3_rpc_url, creditcoin_eth_rpc_url)
    }

    /// Build a single-route [`Config`] from CLI flags (`--single-route`). Used for development.
    #[allow(clippy::too_many_arguments)]
    pub fn single_route(
        bind_host: String,
        bind_port: u16,
        cc3_rpc_url: String,
        creditcoin_eth_rpc_url: String,
        route: ChainRoute,
        p2p: P2pConfig,
    ) -> Self {
        Self {
            bind_host,
            bind_port,
            cc3_rpc_url,
            creditcoin_eth_rpc_url,
            p2p,
            vote_cache: VoteCacheConfig::default(),
            delivery: DeliveryConfig::default(),
            routes: vec![route],
        }
    }

    /// Set of distinct `chain_key`s served by this process — used to filter CC3 events and to
    /// drive metrics labels.
    pub fn chain_keys(&self) -> HashSet<u64> {
        self.routes.iter().map(|r| r.chain_key).collect()
    }
}

impl Default for VoteCacheConfig {
    fn default() -> Self {
        Self {
            ttl_seconds: DEFAULT_VOTE_CACHE_TTL_SECS,
            max_messages: DEFAULT_VOTE_CACHE_MAX_MESSAGES,
        }
    }
}

impl Default for DeliveryConfig {
    fn default() -> Self {
        Self {
            simulate_before_send: DEFAULT_SIMULATE_BEFORE_SEND,
            max_retries: DEFAULT_DELIVERY_MAX_RETRIES,
            gas_multiplier: DEFAULT_GAS_MULTIPLIER,
        }
    }
}

impl Default for P2pConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_P2P_PORT,
            public_addr: None,
            boot_nodes: Vec::new(),
            no_mdns: false,
            identity: None,
        }
    }
}

// ---------------------------------------------------------------------------
// On-disk YAML shape
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct ConfigFile {
    pub bind_host: String,
    pub bind_port: u16,
    #[serde(default)]
    pub p2p: P2pConfigFile,
    #[serde(default)]
    pub vote_cache: VoteCacheConfigFile,
    #[serde(default)]
    pub delivery: DeliveryConfigFile,
    pub routes: Vec<ChainRouteFile>,
}

#[derive(Debug, Default, Deserialize)]
pub struct P2pConfigFile {
    #[serde(default = "default_p2p_port")]
    pub port: u16,
    #[serde(default)]
    pub public_addr: Option<String>,
    #[serde(default)]
    pub boot_nodes: Vec<String>,
    #[serde(default)]
    pub no_mdns: bool,
    #[serde(default)]
    pub identity: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct VoteCacheConfigFile {
    #[serde(default = "default_vote_cache_ttl_secs")]
    pub ttl_seconds: u64,
    #[serde(default = "default_vote_cache_max_messages")]
    pub max_messages: usize,
}

impl Default for VoteCacheConfigFile {
    fn default() -> Self {
        Self {
            ttl_seconds: DEFAULT_VOTE_CACHE_TTL_SECS,
            max_messages: DEFAULT_VOTE_CACHE_MAX_MESSAGES,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct DeliveryConfigFile {
    #[serde(default = "default_simulate_before_send")]
    pub simulate_before_send: bool,
    #[serde(default = "default_delivery_max_retries")]
    pub max_retries: u32,
    #[serde(default = "default_gas_multiplier")]
    pub gas_multiplier: f64,
}

impl Default for DeliveryConfigFile {
    fn default() -> Self {
        Self {
            simulate_before_send: DEFAULT_SIMULATE_BEFORE_SEND,
            max_retries: DEFAULT_DELIVERY_MAX_RETRIES,
            gas_multiplier: DEFAULT_GAS_MULTIPLIER,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ChainRouteFile {
    pub chain_key: u64,
    pub creditcoin_chain_id: u64,
    #[serde(default)]
    pub outbox_address: Option<String>,
    pub destination_rpc_url: String,
    pub inbox_address: String,
    #[serde(default)]
    pub signer_key: Option<String>,
    #[serde(default)]
    pub block_confirmation_depth: u64,
    pub attestor_set: AttestorSetFile,
    #[serde(default)]
    pub threshold_override: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum AttestorSetFile {
    Static { addresses: Vec<String> },
    EvmContract { address: String },
    Cc3ActiveSet { chain_key: u64 },
}

fn default_p2p_port() -> u16 {
    DEFAULT_P2P_PORT
}
fn default_vote_cache_ttl_secs() -> u64 {
    DEFAULT_VOTE_CACHE_TTL_SECS
}
fn default_vote_cache_max_messages() -> usize {
    DEFAULT_VOTE_CACHE_MAX_MESSAGES
}
fn default_simulate_before_send() -> bool {
    DEFAULT_SIMULATE_BEFORE_SEND
}
fn default_delivery_max_retries() -> u32 {
    DEFAULT_DELIVERY_MAX_RETRIES
}
fn default_gas_multiplier() -> f64 {
    DEFAULT_GAS_MULTIPLIER
}

impl ConfigFile {
    fn into_config(self, cc3_rpc_url: String, creditcoin_eth_rpc_url: String) -> Result<Config> {
        if self.routes.is_empty() {
            bail!("config must include at least one entry in `routes`");
        }
        let mut seen: HashSet<u64> = HashSet::new();
        let mut routes = Vec::with_capacity(self.routes.len());
        for r in self.routes {
            if !seen.insert(r.chain_key) {
                bail!("duplicate chain_key {} in config", r.chain_key);
            }
            routes.push(r.into_route()?);
        }

        Ok(Config {
            bind_host: self.bind_host,
            bind_port: self.bind_port,
            cc3_rpc_url,
            creditcoin_eth_rpc_url,
            p2p: P2pConfig {
                port: self.p2p.port,
                public_addr: self.p2p.public_addr,
                boot_nodes: self.p2p.boot_nodes,
                no_mdns: self.p2p.no_mdns,
                identity: self.p2p.identity,
            },
            vote_cache: VoteCacheConfig {
                ttl_seconds: self.vote_cache.ttl_seconds,
                max_messages: self.vote_cache.max_messages,
            },
            delivery: DeliveryConfig {
                simulate_before_send: self.delivery.simulate_before_send,
                max_retries: self.delivery.max_retries,
                gas_multiplier: self.delivery.gas_multiplier,
            },
            routes,
        })
    }
}

impl ChainRouteFile {
    fn into_route(self) -> Result<ChainRoute> {
        let inbox_address = parse_address(&self.inbox_address)
            .with_context(|| format!("invalid inbox_address for chain_key {}", self.chain_key))?;
        let outbox_address = self
            .outbox_address
            .as_deref()
            .map(parse_address)
            .transpose()
            .with_context(|| format!("invalid outbox_address for chain_key {}", self.chain_key))?;

        let attestor_set = match self.attestor_set {
            AttestorSetFile::Static { addresses } => {
                let mut parsed = Vec::with_capacity(addresses.len());
                for raw in addresses {
                    parsed.push(parse_address(&raw).with_context(|| {
                        format!(
                            "invalid attestor address `{raw}` for chain_key {}",
                            self.chain_key
                        )
                    })?);
                }
                if parsed.is_empty() {
                    bail!(
                        "attestor_set.static.addresses must be non-empty for chain_key {}",
                        self.chain_key
                    );
                }
                AttestorSet::Static(parsed)
            }
            AttestorSetFile::EvmContract { address } => {
                let address = parse_address(&address).with_context(|| {
                    format!(
                        "invalid attestor_set.evm_contract.address for chain_key {}",
                        self.chain_key
                    )
                })?;
                AttestorSet::OnChain {
                    source: AttestorSource::Evm { address },
                }
            }
            AttestorSetFile::Cc3ActiveSet { chain_key } => AttestorSet::OnChain {
                source: AttestorSource::Cc3 { chain_key },
            },
        };

        Ok(ChainRoute {
            chain_key: self.chain_key,
            creditcoin_chain_id: self.creditcoin_chain_id,
            outbox_address,
            destination_rpc_url: self.destination_rpc_url,
            inbox_address,
            signer_key: self.signer_key,
            block_confirmation_depth: self.block_confirmation_depth,
            attestor_set,
            threshold_override: self.threshold_override,
        })
    }
}

fn parse_address(raw: &str) -> Result<Address> {
    Address::from_str(raw.trim()).with_context(|| format!("not a valid 0x-hex EVM address: {raw}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_yaml() -> &'static str {
        r#"
bind_host: "0.0.0.0"
bind_port: 3200

p2p:
  port: 9100
  no_mdns: true

vote_cache:
  ttl_seconds: 1800
  max_messages: 1000

delivery:
  simulate_before_send: true
  max_retries: 3
  gas_multiplier: 1.5

routes:
  - chain_key: 2
    creditcoin_chain_id: 102031
    outbox_address: "0x0000000000000000000000000000000000000001"
    destination_rpc_url: "http://localhost:8545"
    inbox_address: "0x0000000000000000000000000000000000000002"
    block_confirmation_depth: 12
    attestor_set:
      kind: static
      addresses:
        - "0x000000000000000000000000000000000000000a"
        - "0x000000000000000000000000000000000000000b"
"#
    }

    #[test]
    fn yaml_round_trip_validates() {
        let file: ConfigFile = serde_yaml::from_str(sample_yaml()).unwrap();
        let cfg = file
            .into_config("ws://cc3:9944".into(), "http://cc3-eth:9933".into())
            .unwrap();
        assert_eq!(cfg.routes.len(), 1);
        assert_eq!(cfg.routes[0].chain_key, 2);
        assert_eq!(cfg.cc3_rpc_url, "ws://cc3:9944");
        assert!(matches!(cfg.routes[0].attestor_set, AttestorSet::Static(_)));
    }

    #[test]
    fn duplicate_chain_keys_rejected() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3200
routes:
  - chain_key: 2
    creditcoin_chain_id: 1
    destination_rpc_url: "http://x"
    inbox_address: "0x0000000000000000000000000000000000000002"
    attestor_set:
      kind: static
      addresses: ["0x000000000000000000000000000000000000000a"]
  - chain_key: 2
    creditcoin_chain_id: 1
    destination_rpc_url: "http://y"
    inbox_address: "0x0000000000000000000000000000000000000003"
    attestor_set:
      kind: static
      addresses: ["0x000000000000000000000000000000000000000a"]
"#;
        let file: ConfigFile = serde_yaml::from_str(yaml).unwrap();
        let err = file
            .into_config("ws://cc3".into(), "http://cc3-eth".into())
            .unwrap_err();
        assert!(err.to_string().contains("duplicate chain_key"));
    }

    #[test]
    fn empty_routes_rejected() {
        let yaml = "bind_host: 0.0.0.0\nbind_port: 3200\nroutes: []\n";
        let file: ConfigFile = serde_yaml::from_str(yaml).unwrap();
        let err = file
            .into_config("ws://cc3".into(), "http://cc3-eth".into())
            .unwrap_err();
        assert!(err.to_string().contains("at least one entry"));
    }

    #[test]
    fn empty_static_attestor_set_rejected() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3200
routes:
  - chain_key: 2
    creditcoin_chain_id: 1
    destination_rpc_url: "http://x"
    inbox_address: "0x0000000000000000000000000000000000000002"
    attestor_set:
      kind: static
      addresses: []
"#;
        let file: ConfigFile = serde_yaml::from_str(yaml).unwrap();
        let err = file
            .into_config("ws://cc3".into(), "http://cc3-eth".into())
            .unwrap_err();
        assert!(err.to_string().contains("must be non-empty"));
    }

    #[test]
    fn invalid_address_rejected() {
        let yaml = r#"
bind_host: "0.0.0.0"
bind_port: 3200
routes:
  - chain_key: 2
    creditcoin_chain_id: 1
    destination_rpc_url: "http://x"
    inbox_address: "not-an-address"
    attestor_set:
      kind: static
      addresses: ["0x000000000000000000000000000000000000000a"]
"#;
        let file: ConfigFile = serde_yaml::from_str(yaml).unwrap();
        let err = file
            .into_config("ws://cc3".into(), "http://cc3-eth".into())
            .unwrap_err();
        assert!(err.to_string().contains("invalid inbox_address"));
    }
}
