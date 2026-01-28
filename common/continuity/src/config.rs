//! Continuity proof generation configuration.
//!
//! This module defines the configuration required to build continuity proofs,
//! including RPC endpoints, chain parameters, and intervals.

/// Configuration for continuity proof generation.
///
/// Use the builder pattern to construct:
///
/// # Examples
///
/// ```rust
/// use continuity::ContinuityConfig;
///
/// let config = ContinuityConfig::builder()
///     .cc3_rpc_url("wss://rpc.creditcoin.network")
///     .eth_rpc_url("https://eth-mainnet.g.alchemy.com/v2/YOUR_KEY")
///     .chain_key(1)
///     .attestation_interval(10)
///     .checkpoint_interval(10)
///     .build();
/// ```
#[derive(Debug, Clone)]
pub struct ContinuityConfig {
    /// CC3 RPC endpoint (WebSocket or HTTP)
    ///
    /// Example: `"wss://rpc.creditcoin.network"` or `"ws://localhost:9944"`
    pub cc3_rpc_url: String,

    /// Source chain (Ethereum/EVM) RPC endpoint
    ///
    /// Example: `"https://eth-mainnet.g.alchemy.com/v2/KEY"` or `"http://localhost:8545"`
    pub eth_rpc_url: String,

    /// Chain key for attestation lookup
    ///
    /// Identifies which source chain this configuration is for.
    /// Each supported chain has a unique key registered in CC3.
    pub chain_key: u64,

    /// Attestation interval (number of source blocks between attestations)
    ///
    /// This should be fetched from the CC3 chain using:
    /// `cc3_client.chain_attestation_interval(chain_key)`
    ///
    /// Typically 10 blocks, but can be configured on-chain.
    pub attestation_interval: u64,

    /// Checkpoint interval (number of attestations between checkpoints)
    ///
    /// This should be fetched from the CC3 chain using:
    /// `cc3_client.chain_checkpoint_interval(chain_key)`
    ///
    /// Default is typically 10 attestations, but it can be changed on-chain.
    pub checkpoint_interval: u64,

    /// Last checkpoint block number (optional optimization)
    ///
    /// If set, queries with block numbers greater than this value can skip
    /// checkpoint checks since no checkpoint exists yet. This is updated via
    /// `CheckpointReached` events and should be fetched at startup using:
    /// `cc3_client.get_last_checkpoint(chain_key)`
    ///
    /// When `None`, checkpoint checks are always performed (slower but always correct).
    pub last_checkpoint_block: Option<u64>,
}

impl ContinuityConfig {
    /// Get the checkpoint interval in blocks (not attestations).
    ///
    /// Checkpoints occur every `checkpoint_interval` attestations, and attestations
    /// occur every `attestation_interval` blocks. So the actual block interval is:
    /// `checkpoint_interval * attestation_interval`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use continuity::ContinuityConfig;
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("ws://mock")
    ///     .eth_rpc_url("http://mock")
    ///     .chain_key(1)
    ///     .attestation_interval(10)
    ///     .checkpoint_interval(10)
    ///     .build();
    ///
    /// assert_eq!(config.checkpoint_block_interval(), 100);
    /// ```
    pub fn checkpoint_block_interval(&self) -> u64 {
        self.checkpoint_interval * self.attestation_interval
    }

    /// Get the maximum range for checkpoint queries around a height.
    ///
    /// Uses `checkpoint_block_interval() * 10` to ensure we get boundaries
    /// even if historical checkpoint intervals were much larger than the current interval.
    /// The GraphQL query limits results to 10 checkpoints per direction, so this is safe.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use continuity::ContinuityConfig;
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("ws://mock")
    ///     .eth_rpc_url("http://mock")
    ///     .chain_key(1)
    ///     .attestation_interval(10)
    ///     .checkpoint_interval(10)
    ///     .build();
    ///
    /// assert_eq!(config.checkpoint_query_max_range(), 1000);
    /// ```
    pub fn checkpoint_query_max_range(&self) -> u64 {
        self.checkpoint_block_interval() * 10
    }
}

impl ContinuityConfig {
    /// Create a configuration builder.
    ///
    /// This is the recommended way to construct a `ContinuityConfig`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use continuity::ContinuityConfig;
    ///
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("wss://rpc.creditcoin.network")
    ///     .eth_rpc_url("https://eth-mainnet.infura.io/v3/KEY")
    ///     .chain_key(1)
    ///     .attestation_interval(10)
    ///     .checkpoint_interval(10)
    ///     .build();
    /// ```
    pub fn builder() -> ConfigBuilder {
        ConfigBuilder::default()
    }
}

/// Builder for [`ContinuityConfig`].
///
/// Provides a fluent API for constructing configuration with method chaining.
///
/// # Examples
///
/// ## Manual Configuration
///
/// ```rust
/// use continuity::ContinuityConfig;
///
/// let config = ContinuityConfig::builder()
///     .cc3_rpc_url("wss://rpc.creditcoin.network")
///     .eth_rpc_url("https://eth-rpc.example.com")
///     .chain_key(1)
///     .attestation_interval(10)
///     .checkpoint_interval(10)
///     .build();
/// ```
///
/// ## With Auto-Fetched Intervals
///
/// ```rust,no_run
/// # async fn example() -> anyhow::Result<()> {
/// use continuity::ContinuityConfig;
///
/// let config = ContinuityConfig::builder()
///     .cc3_rpc_url("wss://rpc.creditcoin.network")
///     .eth_rpc_url("https://eth-rpc.example.com")
///     .chain_key(1)
///     .fetch_intervals()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[derive(Default)]
pub struct ConfigBuilder {
    cc3_rpc_url: Option<String>,
    eth_rpc_url: Option<String>,
    chain_key: Option<u64>,
    attestation_interval: Option<u64>,
    checkpoint_interval: Option<u64>,
    last_checkpoint_block: Option<u64>,
}

impl ConfigBuilder {
    /// Set the CC3 RPC endpoint.
    ///
    /// # Arguments
    ///
    /// * `url` - WebSocket or HTTP endpoint (e.g., "wss://rpc.creditcoin.network")
    pub fn cc3_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.cc3_rpc_url = Some(url.into());
        self
    }

    /// Set the source chain RPC endpoint.
    ///
    /// # Arguments
    ///
    /// * `url` - HTTP or WebSocket endpoint (e.g., "https://eth-mainnet.infura.io/v3/KEY")
    pub fn eth_rpc_url(mut self, url: impl Into<String>) -> Self {
        self.eth_rpc_url = Some(url.into());
        self
    }

    /// Set the chain key.
    ///
    /// # Arguments
    ///
    /// * `key` - The chain identifier (e.g., 1 for Ethereum mainnet)
    pub fn chain_key(mut self, key: u64) -> Self {
        self.chain_key = Some(key);
        self
    }

    /// Set the attestation interval manually.
    ///
    /// # Arguments
    ///
    /// * `interval` - Number of source blocks between attestations
    ///
    /// # Note
    ///
    /// Prefer using [`fetch_intervals`](Self::fetch_intervals) to fetch
    /// this value from the CC3 chain instead of hardcoding it.
    pub fn attestation_interval(mut self, interval: u64) -> Self {
        self.attestation_interval = Some(interval);
        self
    }

    /// Set the checkpoint interval manually.
    ///
    /// # Arguments
    ///
    /// * `interval` - Number of attestations between checkpoints
    ///
    /// # Note
    ///
    /// Prefer using [`fetch_intervals`](Self::fetch_intervals) to fetch
    /// this value from the CC3 chain instead of hardcoding it.
    pub fn checkpoint_interval(mut self, interval: u64) -> Self {
        self.checkpoint_interval = Some(interval);
        self
    }

    /// Set the last checkpoint block number (optional optimization).
    ///
    /// # Arguments
    ///
    /// * `block_number` - The block number of the most recent checkpoint
    ///
    /// When set, queries with block numbers greater than this value can skip
    /// checkpoint checks for better performance. This should be updated when
    /// `CheckpointReached` events are received.
    ///
    /// # Note
    ///
    /// This is an optimization. If `None`, checkpoint checks are always performed
    /// (slower but always correct). Set this value if you're tracking checkpoint
    /// events and want to optimize proof generation.
    pub fn last_checkpoint_block(mut self, block_number: Option<u64>) -> Self {
        self.last_checkpoint_block = block_number;
        self
    }

    /// Build the configuration.
    ///
    /// # Panics
    ///
    /// Panics if any required fields are missing. All fields must be set before calling `build()`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use continuity::ContinuityConfig;
    ///
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("wss://rpc.creditcoin.network")
    ///     .eth_rpc_url("https://eth-rpc.example.com")
    ///     .chain_key(1)
    ///     .attestation_interval(10)
    ///     .checkpoint_interval(10)
    ///     .build();
    /// ```
    pub fn build(self) -> ContinuityConfig {
        ContinuityConfig {
            cc3_rpc_url: self.cc3_rpc_url.expect("cc3_rpc_url is required"),
            eth_rpc_url: self.eth_rpc_url.expect("eth_rpc_url is required"),
            chain_key: self.chain_key.expect("chain_key is required"),
            attestation_interval: self
                .attestation_interval
                .expect("attestation_interval is required"),
            checkpoint_interval: self
                .checkpoint_interval
                .expect("checkpoint_interval is required"),
            last_checkpoint_block: self.last_checkpoint_block,
        }
    }

    /// Fetch both intervals from the CC3 chain and build the config.
    ///
    /// This is the recommended way to build configuration as it ensures both
    /// attestation and checkpoint intervals are fetched dynamically from the chain.
    ///
    /// # Returns
    ///
    /// A fully configured `ContinuityConfig`.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Unable to connect to CC3 RPC
    /// - Intervals are not configured on-chain
    /// - Any required fields are missing
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # async fn example() -> anyhow::Result<()> {
    /// use continuity::ContinuityConfig;
    ///
    /// let config = ContinuityConfig::builder()
    ///     .cc3_rpc_url("wss://rpc.creditcoin.network")
    ///     .eth_rpc_url("https://eth-mainnet.infura.io/v3/KEY")
    ///     .chain_key(1)
    ///     .fetch_intervals()
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn fetch_intervals(self) -> anyhow::Result<ContinuityConfig> {
        use anyhow::Context;
        use cc_client::Client;

        let cc3_rpc_url = self
            .cc3_rpc_url
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cc3_rpc_url is required"))?;
        let chain_key = self
            .chain_key
            .ok_or_else(|| anyhow::anyhow!("chain_key is required"))?;

        let cc3_client = Client::new_read_only(cc3_rpc_url)
            .await
            .context("Failed to create CC3 client")?;

        let attestation_interval = cc3_client
            .chain_attestation_interval(chain_key)
            .await
            .context("Failed to fetch attestation interval")?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Attestation interval not configured for chain {}",
                    chain_key
                )
            })?;

        let checkpoint_interval = cc3_client
            .chain_checkpoint_interval(chain_key)
            .await
            .context("Failed to fetch checkpoint interval")?
            .ok_or_else(|| {
                anyhow::anyhow!("Checkpoint interval not configured for chain {}", chain_key)
            })? as u64;

        Ok(self
            .attestation_interval(attestation_interval)
            .checkpoint_interval(checkpoint_interval)
            .build())
    }
}
