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

/// The subset of `GET /status` this crate acts on. Every field is optional so an archiver
/// that predates the freshness fields still parses: `ready: None` means "unknown".
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct ArchiverStatus {
    pub latest_archived_block: Option<u64>,
    pub ready: Option<bool>,
    pub lag_blocks: Option<u64>,
    pub source_head_age_ms: Option<u64>,
    #[serde(default)]
    pub not_ready_reasons: Vec<String>,
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
    /// The archiver's latest stored height (`GET /roots/latest`), read only when the range came
    /// back incomplete (`404`), so [`Self::is_catching_up_at_tip`] can tell "still fetching the
    /// newest attested blocks" from "has a hole". `None` when not a `404`, when the archive is
    /// empty, or when that lookup itself failed.
    pub latest_archived: Option<u64>,
}

/// How far the archiver may trail the end of a requested range and still count as catching up at
/// the attested tip rather than being down or stalled.
///
/// Following the attested height, the archiver only fetches a range once the attestation that
/// releases it is published, so right after every attestation there is a window in which the
/// range is attested but not yet stored. One attestation releases at most the chain's
/// `MaxCatchup` blocks (500 on every network today), so a lag within that is explained by a single
/// attestation not yet fetched. A larger lag is an outage and must stay a paging 5xx.
pub const ARCHIVER_TIP_LAG_TOLERANCE: u64 = 500;

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

    /// True when the range is incomplete only because the archiver has not stored its newest
    /// blocks yet: its latest height is below the end of the range and within
    /// [`ARCHIVER_TIP_LAG_TOLERANCE`] of it. That is the normal few-second window after an
    /// attestation (a "not ready yet, retry" for the caller), not a fault.
    ///
    /// False when the archiver already holds heights at or beyond the end of the range (the
    /// missing roots are a hole, a real archiver problem), when it trails by more than one
    /// attestation's worth of blocks, or when its latest height is unknown.
    pub fn is_catching_up_at_tip(&self) -> bool {
        self.is_data_unavailable()
            && self.latest_archived.is_some_and(|latest| {
                latest < self.to && self.to - latest <= ARCHIVER_TIP_LAG_TOLERANCE
            })
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
            // An incomplete range is either the archiver still fetching the newest attested blocks
            // or a hole; its latest height tells the two apart. Best effort: a failed lookup
            // leaves it unknown, which keeps the conservative "unavailable" classification.
            let latest_archived = if status == reqwest::StatusCode::NOT_FOUND {
                self.get_latest_block().await.ok().flatten()
            } else {
                None
            };
            return Err(ArchiverStatusError {
                status,
                body,
                from,
                to,
                latest_archived,
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
    /// `GET /status`: liveness plus freshness. Unlike `/roots/latest`, this tells us whether
    /// the archive is *current*, not merely whether the process answers.
    pub async fn status(&self) -> Result<ArchiverStatus> {
        let url = format!("{}/status", self.base_url);
        debug!(archiver_url = %self.base_url, "📡 ➡️  archiver GET /status");
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .context("archiver request failed")?
            .error_for_status()
            .context("archiver returned error status")?;
        response
            .json::<ArchiverStatus>()
            .await
            .context("failed to parse archiver status")
    }

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
        // The archiver must be reachable *and* current. `/status` carries a `ready` verdict
        // (recent source-head sample, within the allowed lag, last flush succeeded); an
        // archiver too old to report it counts as healthy when reachable, as before, so a
        // rollout of proof-gen ahead of the archiver does not flip health.
        let archiver_healthy = match self.archiver.status().await {
            Ok(status) => {
                if status.ready == Some(false) {
                    warn!(
                        archiver_url = %self.archiver.base_url,
                        lag_blocks = ?status.lag_blocks,
                        source_head_age_ms = ?status.source_head_age_ms,
                        reasons = ?status.not_ready_reasons,
                        "archiver reachable but not ready"
                    );
                }
                status.ready.unwrap_or(true)
            }
            Err(err) => {
                warn!(archiver_url = %self.archiver.base_url, %err, "archiver status unavailable");
                false
            }
        };

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-route-per-path HTTP/1.1 server: `/roots?…` answers `roots_status` with `roots_body`,
    /// `/roots/latest` answers `{"latest_block": latest}` and counts how often it was asked.
    async fn fake_archiver(
        roots_status: u16,
        roots_body: &'static str,
        latest: Option<u64>,
    ) -> (String, Arc<AtomicUsize>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let latest_hits = Arc::new(AtomicUsize::new(0));
        let hits = latest_hits.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let hits = hits.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    let (status, body) = if path.starts_with("/roots/latest") {
                        hits.fetch_add(1, Ordering::SeqCst);
                        let latest = latest.map_or("null".to_owned(), |h| h.to_string());
                        (200, format!("{{\"latest_block\":{latest}}}"))
                    } else {
                        (roots_status, roots_body.to_owned())
                    };
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        (url, latest_hits)
    }

    fn status_error(err: &anyhow::Error) -> &ArchiverStatusError {
        anyhow_chain_archiver_status(err).expect("an ArchiverStatusError in the chain")
    }

    #[tokio::test]
    async fn an_incomplete_range_at_the_tip_records_the_latest_height_and_counts_as_catching_up() {
        let (url, latest_hits) = fake_archiver(
            404,
            "incomplete data: expected 30 roots for range 1731..=1760, found 20",
            Some(1_750),
        )
        .await;
        let err = ArchiverClient::new(url)
            .get_roots(1_731, 1_760)
            .await
            .expect_err("a 404 is an error");

        let status = status_error(&err);
        assert_eq!(status.latest_archived, Some(1_750));
        assert!(status.is_catching_up_at_tip());
        assert_eq!(latest_hits.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn an_incomplete_range_below_the_archived_height_is_a_hole_not_catching_up() {
        let (url, _) = fake_archiver(404, "incomplete data", Some(5_000)).await;
        let err = ArchiverClient::new(url)
            .get_roots(1_731, 1_760)
            .await
            .expect_err("a 404 is an error");

        let status = status_error(&err);
        assert_eq!(status.latest_archived, Some(5_000));
        assert!(status.is_data_unavailable());
        assert!(!status.is_catching_up_at_tip());
    }

    #[tokio::test]
    async fn a_range_rejection_does_not_look_up_the_latest_height() {
        let (url, latest_hits) =
            fake_archiver(400, "range too large (max 1000 blocks)", Some(1_750)).await;
        let err = ArchiverClient::new(url)
            .get_roots(1, 5_000)
            .await
            .expect_err("a 400 is an error");

        let status = status_error(&err);
        assert!(status.is_range_rejection());
        assert_eq!(status.latest_archived, None);
        assert_eq!(latest_hits.load(Ordering::SeqCst), 0);
    }
}
