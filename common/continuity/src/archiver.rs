//! Archiver-backed Ethereum provider.
//!
//! Implements `EthRpcProvider` by fetching merkle roots from the archiver's HTTP API
//! instead of hitting the source chain directly. Transaction-level operations (tx bytes,
//! tx hash lookup) are still delegated to a real Ethereum RPC client.

use anyhow::{Context, Result};
use async_trait::async_trait;
use attestor_primitives::block::Block;
use sp_core::H256;
use tracing::{debug, info};

use crate::rpc::{EthRpcProvider, SharedEthProvider};

/// HTTP client for the archiver API.
#[derive(Clone)]
pub struct ArchiverClient {
    base_url: String,
    http: reqwest::Client,
}

/// Response from `GET /roots?from=X&to=Y`.
#[derive(serde::Deserialize)]
struct RootEntry {
    block_number: u64,
    merkle_root: String,
}

/// Response from `GET /roots/latest`.
#[derive(serde::Deserialize)]
struct LatestResponse {
    latest_block: Option<u64>,
}

impl ArchiverClient {
    /// Create a new archiver client pointing at the given base URL (e.g. `http://localhost:8080`).
    pub fn new(base_url: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("failed to build HTTP client");
        Self { base_url, http }
    }

    /// Fetch merkle roots for an inclusive block range [from, to].
    pub async fn get_roots(&self, from: u64, to: u64) -> Result<Vec<(u64, H256)>> {
        let url = format!("{}/roots?from={}&to={}", self.base_url, from, to);
        let entries: Vec<RootEntry> = self
            .http
            .get(&url)
            .send()
            .await
            .context("archiver request failed")?
            .error_for_status()
            .context("archiver returned error status")?
            .json()
            .await
            .context("failed to parse archiver response")?;

        entries
            .into_iter()
            .map(|e| {
                let root = parse_h256(&e.merkle_root)
                    .with_context(|| format!("bad root for block {}", e.block_number))?;
                Ok((e.block_number, root))
            })
            .collect()
    }

    /// Get the latest archived block number.
    pub async fn get_latest_block(&self) -> Result<Option<u64>> {
        let url = format!("{}/roots/latest", self.base_url);
        let resp: LatestResponse = self
            .http
            .get(&url)
            .send()
            .await
            .context("archiver request failed")?
            .error_for_status()
            .context("archiver returned error status")?
            .json()
            .await
            .context("failed to parse archiver response")?;
        Ok(resp.latest_block)
    }
}

/// An `EthRpcProvider` that fetches block roots from the archiver and delegates
/// transaction-level operations to a real Ethereum client.
pub struct ArchiverEthProvider {
    archiver: ArchiverClient,
    eth_fallback: SharedEthProvider,
}

impl ArchiverEthProvider {
    pub fn new(archiver_url: String, eth_fallback: SharedEthProvider) -> Self {
        Self {
            archiver: ArchiverClient::new(archiver_url),
            eth_fallback,
        }
    }
}

#[async_trait]
impl EthRpcProvider for ArchiverEthProvider {
    async fn build_continuity_blocks(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
    ) -> Result<Vec<Block>> {
        debug!(start, end, "fetching roots from archiver");

        let roots = self.archiver.get_roots(start, end).await.with_context(|| {
            format!("failed to get roots from archiver for range {start}..{end}")
        })?;

        if roots.is_empty() {
            anyhow::bail!("archiver returned no roots for range {start}..{end}");
        }

        let expected_count = (end - start + 1) as usize;
        anyhow::ensure!(
            roots.len() == expected_count,
            "archiver returned {} roots but expected {} for range {start}..={end}",
            roots.len(),
            expected_count,
        );

        let mut blocks = Vec::with_capacity(roots.len());
        let mut prev_digest = lower_digest;

        for (height, root) in roots {
            let block = Block::new_from_prev_digest(height, root, prev_digest);
            prev_digest = block.digest();
            blocks.push(block);
        }

        info!(
            count = blocks.len(),
            start = blocks.first().map(|b| b.n()),
            end = blocks.last().map(|b| b.n()),
            "built continuity blocks from archiver roots"
        );

        Ok(blocks)
    }

    async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
        self.eth_fallback.get_block_tx_bytes(block_number).await
    }

    async fn get_tx_hash_by_index(&self, block_number: u64, tx_index: u64) -> Result<Option<H256>> {
        self.eth_fallback
            .get_tx_hash_by_index(block_number, tx_index)
            .await
    }

    async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<Option<(u64, u64)>> {
        self.eth_fallback.get_tx_position_by_hash(tx_hash).await
    }

    async fn get_last_block(&self) -> Result<u64> {
        // Always query the real chain tip — the archiver is always behind the actual chain head,
        // so using archiver's tip would incorrectly reject valid blocks.
        self.eth_fallback.get_last_block().await
    }

    async fn get_chain_id(&self) -> Result<u64> {
        self.eth_fallback.get_chain_id().await
    }
}

fn parse_h256(s: &str) -> Result<H256> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).with_context(|| format!("invalid hex: {s}"))?;
    anyhow::ensure!(bytes.len() == 32, "expected 32 bytes, got {}", bytes.len());
    Ok(H256::from_slice(&bytes))
}
