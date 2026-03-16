//! Archiver-backed Ethereum provider.
//!
//! Implements `EthRpcProvider` by fetching merkle roots from the archiver's HTTP API
//! instead of hitting the source chain directly. Transaction-level operations (tx bytes,
//! tx hash lookup) are still delegated to a real Ethereum RPC client.

use anyhow::{Context, Result};
use async_trait::async_trait;
use attestor_primitives::block::{Block, ContinuityProof};
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

/// Response from `GET /proof-input?from=X&to=Y`.
#[derive(serde::Deserialize)]
struct ProofInputResponse {
    lower_endpoint_digest: String,
    roots: Vec<RootEntry>,
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

    /// Build a `ContinuityProof` directly from the archiver for the given query blocks.
    ///
    /// The proof spans from `min_query` to `upper_checkpoint`:
    /// - `upper_checkpoint = ceil(max_query / checkpoint_interval) * checkpoint_interval`
    /// - `lower_endpoint_digest` = chained digest at `min_query - 1` (computed by archiver)
    /// - `roots` = merkle roots from `min_query` to `upper_checkpoint`
    pub async fn build_continuity_proof(
        &self,
        min_query: u64,
        max_query: u64,
        checkpoint_interval: u64,
    ) -> Result<ContinuityProof> {
        anyhow::ensure!(checkpoint_interval > 0, "checkpoint_interval must be > 0");

        let upper_checkpoint = max_query.div_ceil(checkpoint_interval) * checkpoint_interval;
        let proof_from = min_query;
        let proof_to = upper_checkpoint;

        info!(
            min_query,
            max_query,
            upper_checkpoint,
            proof_from,
            proof_to,
            roots_count = proof_to - proof_from + 1,
            "building continuity proof from archiver"
        );

        let url = format!(
            "{}/proof-input?from={}&to={}",
            self.base_url, proof_from, proof_to
        );
        let resp: ProofInputResponse = self
            .http
            .get(&url)
            .send()
            .await
            .context("archiver /proof-input request failed")?
            .error_for_status()
            .context("archiver /proof-input returned error status")?
            .json()
            .await
            .context("failed to parse archiver /proof-input response")?;

        let lower_endpoint_digest = parse_h256(&resp.lower_endpoint_digest)
            .context("bad lower_endpoint_digest from archiver")?;

        let roots: Vec<H256> = resp
            .roots
            .into_iter()
            .map(|e| {
                parse_h256(&e.merkle_root)
                    .with_context(|| format!("bad root for block {}", e.block_number))
            })
            .collect::<Result<Vec<_>>>()?;

        let expected_count = (proof_to - proof_from + 1) as usize;
        anyhow::ensure!(
            roots.len() == expected_count,
            "archiver returned {} roots but expected {} (from={}, to={})",
            roots.len(),
            expected_count,
            proof_from,
            proof_to,
        );

        info!(
            proof_from,
            proof_to,
            roots_count = roots.len(),
            lower_endpoint_digest = ?lower_endpoint_digest,
            "built continuity proof from archiver"
        );

        Ok(ContinuityProof::new(lower_endpoint_digest, roots))
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
        match self.archiver.get_latest_block().await {
            Ok(Some(height)) => Ok(height),
            _ => {
                debug!("archiver latest block unavailable, falling back to eth client");
                self.eth_fallback.get_last_block().await
            }
        }
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
