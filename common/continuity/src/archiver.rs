//! Archiver-backed Ethereum provider.
//!
//! Implements `EthRpcProvider` by fetching merkle roots from the archiver's HTTP API
//! instead of hitting the source chain directly. Transaction-level operations (tx bytes,
//! tx hash lookup) are still delegated to a real Ethereum RPC client.

use anyhow::{Context, Result};
use async_trait::async_trait;
use attestor_primitives::block::Block;
use sp_core::H256;
use std::time::Instant;
use tracing::{debug, info, warn};

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

/// A non-2xx response from the archiver's HTTP API.
///
/// Kept as a typed error rather than collapsing straight into `anyhow!` so callers can tell three
/// cases apart, because they need three different answers:
///
/// - **Range rejection** (`400`) — a request the archiver will never serve, such as a block range
///   wider than its `MAX_API_RANGE`, or `to < from`. A caller mistake; must not surface as a 5xx,
///   which both misleads the client and pages an operator for a request that could never have
///   succeeded.
/// - **Data unavailable** (`404`) — the range is perfectly valid, but the archiver does not hold
///   every root in it yet (`incomplete data: expected N roots ... found M`). That is a *temporary*
///   availability gap while it catches up or backfills, so the caller should be told to retry and
///   an operator should still see it: a persistent 404 means a real archiver gap.
/// - **Archiver fault** (`5xx`) — a genuine server-side failure, left to the caller's fallback.
///
/// The 400/404 split matters: collapsing both into "client rejection" tells a caller its range was
/// bad when the range was fine, marks a recoverable condition non-retriable, and downgrades the log
/// out of the range an operator is paged on.
#[derive(Debug, thiserror::Error)]
#[error("archiver rejected GET /roots for range {from}..{to} with {status}: {body}")]
pub struct ArchiverStatusError {
    pub status: reqwest::StatusCode,
    pub body: String,
    pub from: u64,
    pub to: u64,
}

impl ArchiverStatusError {
    /// True when the archiver refused the request itself and always will — a 4xx that is not a
    /// `404`. Retrying an identical request cannot help.
    ///
    /// `404` is deliberately excluded: see [`Self::is_data_unavailable`].
    pub fn is_range_rejection(&self) -> bool {
        self.status.is_client_error() && self.status != reqwest::StatusCode::NOT_FOUND
    }

    /// True when the archiver accepted the range but cannot serve it *yet* — a `404`, which its
    /// `/roots` handler returns when the store holds fewer roots than the range asks for. The same
    /// request can succeed once the archiver has caught up, so this is retriable.
    pub fn is_data_unavailable(&self) -> bool {
        self.status == reqwest::StatusCode::NOT_FOUND
    }
}

/// Find an [`ArchiverStatusError`] anywhere in an `anyhow` error chain.
pub fn anyhow_chain_archiver_status(err: &anyhow::Error) -> Option<&ArchiverStatusError> {
    err.chain()
        .find_map(|cause| cause.downcast_ref::<ArchiverStatusError>())
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
        let span = (to.saturating_sub(from)).saturating_add(1);
        debug!(
            archiver_url = %self.base_url,
            from,
            to,
            span,
            "📡 ➡️  archiver GET /roots"
        );

        let started = Instant::now();
        let response = self.http.get(&url).send().await.with_context(|| {
            let elapsed_ms = started.elapsed().as_millis();
            warn!(
                archiver_url = %self.base_url,
                from,
                to,
                span,
                duration_ms = elapsed_ms,
                "📡 ❌ archiver GET /roots transport error"
            );
            "archiver request failed"
        })?;

        let status = response.status();
        if !status.is_success() {
            let elapsed_ms = started.elapsed().as_millis();
            // Read the body before discarding the response: the archiver puts the actionable
            // detail there (e.g. "range too large (max 1000 blocks)"), and `error_for_status`
            // would throw it away, leaving callers only a bare status to reason about.
            let body = response.text().await.unwrap_or_default();
            let body = body.trim().to_string();
            warn!(
                archiver_url = %self.base_url,
                from,
                to,
                span,
                status = %status,
                duration_ms = elapsed_ms,
                detail = %body,
                "📡 ❌ archiver GET /roots non-success status"
            );
            return Err(ArchiverStatusError {
                status,
                body,
                from,
                to,
            }
            .into());
        }

        let entries: Vec<RootEntry> = response
            .json()
            .await
            .context("failed to parse archiver response")?;
        let elapsed_ms = started.elapsed().as_millis();
        info!(
            archiver_url = %self.base_url,
            from,
            to,
            span,
            count = entries.len(),
            status = %status,
            duration_ms = elapsed_ms,
            "📡 ✅ archiver GET /roots completed"
        );

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
        debug!(
            archiver_url = %self.base_url,
            "📡 ➡️  archiver GET /roots/latest"
        );
        let started = Instant::now();
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .context("archiver request failed")?
            .error_for_status()
            .context("archiver returned error status")?;
        let status = response.status();
        let resp: LatestResponse = response
            .json()
            .await
            .context("failed to parse archiver response")?;
        let elapsed_ms = started.elapsed().as_millis();
        info!(
            archiver_url = %self.base_url,
            latest_block = ?resp.latest_block,
            status = %status,
            duration_ms = elapsed_ms,
            "📡 ✅ archiver GET /roots/latest completed"
        );
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
        debug!(start, end, "🔧 📡 fetching roots from archiver");

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

        // Validate that every returned entry sits at the expected height
        // (block_number == start + i). The count check above only verifies the array
        // length — the archiver could still return entries in a different order, with
        // gaps, or with duplicates and the count would happen to line up if mirrored
        // by extras elsewhere. EVM continuity proofs incorporate height-ordered roots
        // into the digest chain, so an off-by-one or reordering silently corrupts the
        // proof. Reject the whole response on any mismatch and let the caller decide
        // whether to retry or fall back to the EVM RPC.
        for (i, (height, root)) in roots.into_iter().enumerate() {
            let expected_height = start + i as u64;
            anyhow::ensure!(
                height == expected_height,
                "archiver returned out-of-order or off-by-one entry at index {i}: \
                 expected block {expected_height}, got {height} (range {start}..={end})"
            );
            let block = Block::new_from_prev_digest(height, root, prev_digest);
            prev_digest = block.digest();
            blocks.push(block);
        }

        info!(
            count = blocks.len(),
            start = blocks.first().map(|b| b.n()),
            end = blocks.last().map(|b| b.n()),
            "🔧 🧱 built continuity blocks from archiver roots"
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

    async fn get_block_tx_bytes_and_tx_hash(
        &self,
        block_number: u64,
        tx_index: u64,
    ) -> Result<(Vec<Vec<u8>>, Option<H256>)> {
        self.eth_fallback
            .get_block_tx_bytes_and_tx_hash(block_number, tx_index)
            .await
    }

    async fn get_block_tx_data(&self, block_number: u64) -> Result<Vec<(H256, Vec<u8>)>> {
        self.eth_fallback.get_block_tx_data(block_number).await
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

    async fn is_healthy(&self) -> Result<bool> {
        // Check both the archiver and the fallback RPC for health.
        let archiver_healthy = self
            .archiver
            .get_latest_block()
            .await
            .map(|_| true)
            .unwrap_or(false);

        let eth_healthy = self.eth_fallback.is_healthy().await.unwrap_or(false);

        Ok(archiver_healthy && eth_healthy)
    }
}

fn parse_h256(s: &str) -> Result<H256> {
    let s = s.strip_prefix("0x").unwrap_or(s);
    let bytes = hex::decode(s).with_context(|| format!("invalid hex: {s}"))?;
    anyhow::ensure!(bytes.len() == 32, "expected 32 bytes, got {}", bytes.len());
    Ok(H256::from_slice(&bytes))
}
