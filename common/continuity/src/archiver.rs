//! Archiver-backed Ethereum provider.
//!
//! Implements `EthRpcProvider` by fetching merkle roots from the archiver's HTTP API
//! instead of hitting the source chain directly. Transaction-level operations (tx bytes,
//! tx hash lookup) are still delegated to a real Ethereum RPC client.
//!
//! When the archiver cannot serve a range, the provider builds it from the source RPC instead of
//! failing the request: only the missing tail while the archiver is still storing the newest
//! attested blocks (see [`ArchiverStatusError::is_catching_up_at_tip`]), the whole range for any
//! other archiver failure. Only a range rejection is passed through to the caller. The first
//! fallback that covers for a failed archiver is logged at ERROR, so it still gets fixed; the rest
//! of the outage logs at WARN until the archiver serves a range again.

use anyhow::{Context, Result};
use async_trait::async_trait;
use attestor_primitives::block::Block;
use sp_core::H256;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

use crate::rpc::{EthRpcProvider, SharedEthProvider};

/// How long to wait for a TCP connection to the archiver.
const ARCHIVER_CONNECT_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a `/roots` or `/roots/latest` request may take in all.
///
/// The archiver answers these from its local store, in milliseconds even for its largest range,
/// and a request that misses this deadline is built from the source RPC instead. A short deadline
/// keeps a hung archiver from adding its whole wait to every proof request before that fallback
/// starts.
const ARCHIVER_ROOTS_TIMEOUT: Duration = Duration::from_secs(5);

/// How long any other archiver request (`/status`) may take in all.
const ARCHIVER_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// HTTP client for the archiver API.
#[derive(Clone)]
pub struct ArchiverClient {
    base_url: String,
    http: reqwest::Client,
    /// Deadline for `/roots` and `/roots/latest`; see [`ARCHIVER_ROOTS_TIMEOUT`].
    roots_timeout: Duration,
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
            .connect_timeout(ARCHIVER_CONNECT_TIMEOUT)
            .timeout(ARCHIVER_REQUEST_TIMEOUT)
            .build()
            .expect("failed to build HTTP client");
        Self {
            base_url,
            http,
            roots_timeout: ARCHIVER_ROOTS_TIMEOUT,
        }
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
        let response = self
            .http
            .get(&url)
            .timeout(self.roots_timeout)
            .send()
            .await
            .with_context(|| {
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
            .timeout(self.roots_timeout)
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

/// Why roots were built from the source RPC instead of being served by the archiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcFallbackReason {
    /// The archiver is still storing the newest blocks of a freshly attested range: the normal
    /// few-second window after an attestation. Only the missing tail comes from the source RPC.
    CatchingUp,
    /// The archiver holds heights past the range but not every root in it: a gap in the archive.
    Hole,
    /// The archiver trails the range by more than one attestation's worth of blocks.
    Stalled,
    /// The range came back incomplete and the archiver's latest height could not be read.
    UnknownHeight,
    /// The archiver answered with a server error, or another non-success status that is not a
    /// range rejection.
    ArchiverFault,
    /// The archiver could not be reached or did not answer in time.
    Unreachable,
    /// The archiver answered with roots that cannot be used: unparseable, the wrong number of
    /// them, or out of order.
    BadResponse,
}

impl RpcFallbackReason {
    /// True for the expected post-attestation window, false when the archiver has failed and
    /// the fallback is covering for it.
    pub fn is_archiver_failure(self) -> bool {
        !matches!(self, Self::CatchingUp)
    }
}

/// How a source-RPC fallback ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RpcFallbackOutcome {
    /// The roots were built from the source RPC and the request was served.
    Served,
    /// The fallback failed too; the caller got the archiver's error.
    Failed,
}

/// Called once per request that falls back to the source RPC, with its reason and outcome, so the
/// embedding service can count them. A request that succeeds through the fallback still means the
/// archiver did not serve it, and a steady rate of `CatchingUp` means it keeps losing the race to
/// the prover.
pub type RpcFallbackObserver = Arc<dyn Fn(RpcFallbackReason, RpcFallbackOutcome) + Send + Sync>;

/// Which archiver failure `err` is, or `None` when the source RPC must not cover it.
///
/// A range rejection (a 4xx other than `404`) is the only failure left to the caller: the request
/// itself is too large or malformed, and building it from the source RPC instead would turn an
/// oversized request into the most expensive one the prover can serve.
fn fallback_reason(err: &anyhow::Error) -> Option<RpcFallbackReason> {
    if let Some(status) = anyhow_chain_archiver_status(err) {
        if status.is_range_rejection() {
            return None;
        }
        if status.is_catching_up_at_tip() {
            return Some(RpcFallbackReason::CatchingUp);
        }
        if status.is_data_unavailable() {
            return Some(match status.latest_archived {
                Some(latest) if latest >= status.to => RpcFallbackReason::Hole,
                Some(_) => RpcFallbackReason::Stalled,
                None => RpcFallbackReason::UnknownHeight,
            });
        }
        return Some(RpcFallbackReason::ArchiverFault);
    }
    // No status: either the request never completed (connect, timeout, dropped body), or the
    // archiver's 200 carried roots we cannot use (a JSON decode error, bad hex, a wrong count or
    // order — the last three carry no `reqwest` error at all).
    let transport = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<reqwest::Error>());
    Some(match transport {
        Some(e) if !e.is_decode() => RpcFallbackReason::Unreachable,
        _ => RpcFallbackReason::BadResponse,
    })
}

/// An `EthRpcProvider` that fetches block roots from the archiver and delegates
/// transaction-level operations to a real Ethereum client.
pub struct ArchiverEthProvider {
    archiver: ArchiverClient,
    eth_fallback: SharedEthProvider,
    fallback_observer: Option<RpcFallbackObserver>,
    /// Set by the first request the archiver fails, cleared by the next one it serves, so an
    /// outage logs one ERROR when it starts rather than one per request.
    archiver_failing: AtomicBool,
}

impl ArchiverEthProvider {
    pub fn new(archiver_url: String, eth_fallback: SharedEthProvider) -> Self {
        Self {
            archiver: ArchiverClient::new(archiver_url),
            eth_fallback,
            fallback_observer: None,
            archiver_failing: AtomicBool::new(false),
        }
    }

    /// Report every source-RPC fallback's outcome to `observer`.
    pub fn with_fallback_observer(mut self, observer: RpcFallbackObserver) -> Self {
        self.fallback_observer = Some(observer);
        self
    }

    fn observe_fallback(&self, reason: RpcFallbackReason, outcome: RpcFallbackOutcome) {
        if let Some(observer) = &self.fallback_observer {
            observer(reason, outcome);
        }
    }

    /// Record that the archiver failed a request; true if it was serving until now, so this is
    /// the start of an outage.
    fn archiver_started_failing(&self) -> bool {
        !self.archiver_failing.swap(true, Ordering::Relaxed)
    }

    /// Record that the archiver answered as it should (a served range, or catching up at the
    /// tip), logging the end of an outage.
    fn archiver_answered(&self) {
        if self.archiver_failing.swap(false, Ordering::Relaxed) {
            info!(
                archiver_url = %self.archiver.base_url,
                "🔧 ✅ archiver is serving roots again"
            );
        }
    }

    /// Build `start..=end` while the archiver holds only up to `latest_archived` of it: the
    /// stored prefix from the archiver, the missing tail from the source RPC, chained into one
    /// digest sequence.
    ///
    /// Only the tail goes to the source RPC, so catching up costs a few blocks of RPC work rather
    /// than the whole range. `latest_archived` comes from an archiver that is catching up at the
    /// tip, so the tail is at most [`ARCHIVER_TIP_LAG_TOLERANCE`] blocks. If the archiver cannot
    /// serve the prefix either, the whole range comes from the source RPC.
    ///
    /// The prefix is clamped to `end`, so a `latest_archived` at or past the end of the range
    /// never fetches roots outside it and leaves no tail to build.
    async fn build_with_source_tail(
        &self,
        lower_digest: H256,
        start: u64,
        end: u64,
        latest_archived: u64,
    ) -> Result<Vec<Block>> {
        let prefix_end = latest_archived.min(end);
        let mut blocks = if prefix_end >= start {
            match self
                .archiver
                .get_roots(start, prefix_end)
                .await
                .and_then(|roots| blocks_from_roots(lower_digest, start, prefix_end, roots))
            {
                Ok(prefix) => prefix,
                Err(err) => {
                    warn!(
                        start,
                        end,
                        latest_archived,
                        err = %format!("{err:#}"),
                        "🔧 archiver could not serve the stored prefix either; building the whole \
                         range from the source RPC"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        let (tail_start, tail_lower_digest) = blocks
            .last()
            .map_or((start, lower_digest), |last| (last.n() + 1, last.digest()));
        if tail_start > end {
            // The archiver served the whole range; there is nothing left to build.
            return Ok(blocks);
        }
        let tail = self
            .source_blocks(tail_lower_digest, tail_start, end)
            .await?;
        blocks.extend(tail);
        Ok(blocks)
    }

    /// Build `start..=end` from the source RPC, checking that it holds exactly the blocks asked
    /// for, in order.
    ///
    /// Requests for the same range are not merged here: the eth client's block cache serves the
    /// blocks to every request after the first that fetched them.
    async fn source_blocks(&self, lower_digest: H256, start: u64, end: u64) -> Result<Vec<Block>> {
        let blocks = self
            .eth_fallback
            .build_continuity_blocks(lower_digest, start, end)
            .await?;
        let expected = (end - start + 1) as usize;
        anyhow::ensure!(
            blocks.len() == expected,
            "source RPC built {} blocks but expected {expected} for range {start}..={end}",
            blocks.len(),
        );
        for (i, block) in blocks.iter().enumerate() {
            let expected_height = start + i as u64;
            anyhow::ensure!(
                block.n() == expected_height,
                "source RPC built an out-of-order block at index {i}: expected block \
                 {expected_height}, got {} (range {start}..={end})",
                block.n()
            );
        }
        Ok(blocks)
    }
}

/// Chain archiver roots for `start..=end` into continuity blocks starting from `lower_digest`.
fn blocks_from_roots(
    lower_digest: H256,
    start: u64,
    end: u64,
    roots: Vec<(u64, H256)>,
) -> Result<Vec<Block>> {
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

    Ok(blocks)
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

        // A 200 with unusable roots (wrong count, out of order) is an archiver failure like any
        // other, so it goes through the same fallback as a failed request.
        let archiver_err = match self
            .archiver
            .get_roots(start, end)
            .await
            .and_then(|roots| blocks_from_roots(lower_digest, start, end, roots))
        {
            Ok(blocks) => {
                self.archiver_answered();
                info!(
                    count = blocks.len(),
                    start = blocks.first().map(|b| b.n()),
                    end = blocks.last().map(|b| b.n()),
                    "🔧 🧱 built continuity blocks from archiver roots"
                );
                return Ok(blocks);
            }
            Err(err) => err.context(format!(
                "failed to get roots from archiver for range {start}..{end}"
            )),
        };

        let Some(reason) = fallback_reason(&archiver_err) else {
            return Err(archiver_err);
        };
        let latest_archived =
            anyhow_chain_archiver_status(&archiver_err).and_then(|status| status.latest_archived);
        // Only an archiver failure counts toward an outage; catching up at the tip is the archiver
        // answering as designed.
        let outage_started = if reason.is_archiver_failure() {
            self.archiver_started_failing()
        } else {
            self.archiver_answered();
            false
        };

        // Catching up, only the blocks the archiver has not stored yet come from the source RPC.
        // Any other failure leaves nothing in the archiver's answer to trust, so the whole range
        // does.
        let result = match (reason, latest_archived) {
            (RpcFallbackReason::CatchingUp, Some(latest)) => {
                self.build_with_source_tail(lower_digest, start, end, latest)
                    .await
            }
            _ => self.source_blocks(lower_digest, start, end).await,
        };

        match result {
            Ok(blocks) => {
                self.observe_fallback(reason, RpcFallbackOutcome::Served);
                if outage_started {
                    // ERROR, as an archiver failure was before the fallback existed: the request
                    // is served, but the archiver is broken and someone must look at it. Once per
                    // outage, not per request: the fallback counter tracks how many it covers.
                    error!(
                        ?reason,
                        start,
                        end,
                        ?latest_archived,
                        archiver_err = %format!("{archiver_err:#}"),
                        "🔧 ⚠️ archiver failed; serving ranges from the source RPC until it recovers"
                    );
                } else if reason.is_archiver_failure() {
                    warn!(
                        ?reason,
                        start,
                        end,
                        ?latest_archived,
                        archiver_err = %format!("{archiver_err:#}"),
                        "🔧 ⚠️ archiver still failing; served the range from the source RPC instead"
                    );
                } else {
                    info!(
                        start,
                        end,
                        ?latest_archived,
                        "🔧 🧱 archiver still catching up; built the missing tail from the source RPC"
                    );
                }
                Ok(blocks)
            }
            Err(fallback_err) => {
                self.observe_fallback(reason, RpcFallbackOutcome::Failed);
                if reason.is_archiver_failure() {
                    // ERROR on every request: this one fails, as it did before the fallback.
                    error!(
                        ?reason,
                        start,
                        end,
                        ?latest_archived,
                        archiver_err = %format!("{archiver_err:#}"),
                        fallback_err = %format!("{fallback_err:#}"),
                        "🔧 ❌ archiver failed and the source RPC fallback failed too"
                    );
                } else {
                    warn!(
                        start,
                        end,
                        ?latest_archived,
                        fallback_err = %format!("{fallback_err:#}"),
                        "🔧 ❌ archiver still catching up and the source RPC fallback failed"
                    );
                }
                // Answer with the archiver's error, so the caller gets the same classification
                // (a retriable "catching up", "data unavailable", ...) it got before the fallback
                // existed, rather than whatever the source RPC failed with.
                Err(archiver_err)
            }
        }
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

        // Keep the cause: reducing it to `false` left "eth RPC chain N is unhealthy" with no
        // record of whether the tip read timed out, errored, or the provider was unreachable.
        let eth_healthy = match self.eth_fallback.is_healthy().await {
            Ok(healthy) => healthy,
            Err(err) => {
                warn!(archiver_url = %self.archiver.base_url, err = %format!("{err:#}"), "source tip probe failed");
                false
            }
        };

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
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-route-per-path HTTP/1.1 server: `/roots?…` answers `roots_status` with `roots_body`,
    /// `/roots/latest` answers `{"latest_block": latest}` and counts how often it was asked.
    async fn fake_archiver(
        roots_status: u16,
        roots_body: &'static str,
        latest: Option<u64>,
    ) -> (String, Arc<AtomicUsize>) {
        let latest_hits = Arc::new(AtomicUsize::new(0));
        let hits = latest_hits.clone();
        let url = serve(Arc::new(move |path: &str| -> (u16, String) {
            if path.starts_with("/roots/latest") {
                hits.fetch_add(1, Ordering::SeqCst);
                let latest = latest.map_or("null".to_owned(), |h| h.to_string());
                (200, format!("{{\"latest_block\":{latest}}}"))
            } else {
                (roots_status, roots_body.to_owned())
            }
        }))
        .await;
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

    // ── Source-RPC fallback ─────────────────────────────────────────────────

    use crate::mocks::MockEthRpcProvider;
    use std::time::Duration;

    /// The root the fake archiver serves for `n`: the same one [`MockEthRpcProvider`] builds, so a
    /// range assembled from both sources must chain to the digests of an all-RPC build.
    fn archived_root(n: u64) -> H256 {
        H256::from_low_u64_be(n + 1000)
    }

    /// A `/roots` response body for `heights`, in the order given.
    fn roots_body(heights: impl Iterator<Item = u64>) -> String {
        let entries: Vec<String> = heights
            .map(|n| {
                format!(
                    "{{\"block_number\":{n},\"merkle_root\":\"0x{}\"}}",
                    hex::encode(archived_root(n).as_bytes())
                )
            })
            .collect();
        format!("[{}]", entries.join(","))
    }

    /// The `from` and `to` query parameters of a `/roots` request path.
    fn range_of(path: &str) -> (u64, u64) {
        let param = |name: &str| -> u64 {
            path.split(['?', '&'])
                .find_map(|kv| kv.strip_prefix(name)?.strip_prefix('='))
                .and_then(|v| v.parse().ok())
                .expect("from/to query parameters")
        };
        (param("from"), param("to"))
    }

    type Handler = Arc<dyn Fn(&str) -> (u16, String) + Send + Sync>;

    /// An HTTP/1.1 server answering every request with `handler(path)`.
    async fn serve(handler: Handler) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let handler = handler.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = stream.read(&mut buf).await.unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request.split_whitespace().nth(1).unwrap_or("");
                    let (status, body) = handler(path);
                    let response = format!(
                        "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        url
    }

    /// An archiver that behaves like the real `/roots` handler: it serves any range ending at or
    /// below `stored_up_to` and answers `404 incomplete data` for anything past it.
    /// `/roots/latest` reports `latest` (normally `Some(stored_up_to)`; a larger value fakes a
    /// hole, `None` an empty or unreadable height).
    async fn archiver_storing_up_to(stored_up_to: u64, latest: Option<u64>) -> String {
        serve(Arc::new(move |path: &str| -> (u16, String) {
            if path.starts_with("/roots/latest") {
                let latest = latest.map_or("null".to_owned(), |h| h.to_string());
                return (200, format!("{{\"latest_block\":{latest}}}"));
            }
            let (from, to) = range_of(path);
            if to <= stored_up_to {
                (200, roots_body(from..=to))
            } else {
                (404, "incomplete data".to_owned())
            }
        }))
        .await
    }

    /// An archiver that answers every request with `status` and `body`.
    async fn archiver_answering(status: u16, body: &'static str) -> String {
        serve(Arc::new(move |_: &str| (status, body.to_owned()))).await
    }

    /// An archiver that accepts connections and never answers.
    async fn hung_archiver() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut open = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                open.push(stream);
            }
        });
        url
    }

    /// A URL nothing listens on: the port was bound and released, so connecting is refused.
    async fn unreachable_archiver() -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        drop(listener);
        url
    }

    /// A source RPC that builds the same chain as [`MockEthRpcProvider`], counting the ranges it
    /// is asked for, optionally failing.
    #[derive(Default)]
    struct CountingSource {
        ranges: Mutex<Vec<(u64, u64)>>,
        fail: bool,
    }

    impl CountingSource {
        fn ranges(&self) -> Vec<(u64, u64)> {
            self.ranges.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EthRpcProvider for CountingSource {
        async fn build_continuity_blocks(
            &self,
            lower_digest: H256,
            start: u64,
            end: u64,
        ) -> Result<Vec<Block>> {
            self.ranges.lock().unwrap().push((start, end));
            anyhow::ensure!(!self.fail, "source RPC unavailable");
            MockEthRpcProvider
                .build_continuity_blocks(lower_digest, start, end)
                .await
        }
        async fn get_block_tx_bytes(&self, block_number: u64) -> Result<Vec<Vec<u8>>> {
            MockEthRpcProvider.get_block_tx_bytes(block_number).await
        }
        async fn get_tx_hash_by_index(&self, block: u64, index: u64) -> Result<Option<H256>> {
            MockEthRpcProvider.get_tx_hash_by_index(block, index).await
        }
        async fn get_tx_position_by_hash(&self, tx_hash: H256) -> Result<Option<(u64, u64)>> {
            MockEthRpcProvider.get_tx_position_by_hash(tx_hash).await
        }
        async fn get_last_block(&self) -> Result<u64> {
            MockEthRpcProvider.get_last_block().await
        }
        async fn get_chain_id(&self) -> Result<u64> {
            MockEthRpcProvider.get_chain_id().await
        }
        async fn is_healthy(&self) -> Result<bool> {
            Ok(true)
        }
    }

    type Observed = Arc<Mutex<Vec<(RpcFallbackReason, RpcFallbackOutcome)>>>;

    /// A provider over `url` and `source` that records every fallback's reason and outcome.
    fn provider(url: String, source: Arc<CountingSource>) -> (ArchiverEthProvider, Observed) {
        let observed: Observed = Arc::new(Mutex::new(Vec::new()));
        let seen = observed.clone();
        let provider = ArchiverEthProvider::new(url, source).with_fallback_observer(Arc::new(
            move |reason, outcome| seen.lock().unwrap().push((reason, outcome)),
        ));
        (provider, observed)
    }

    fn observed(observed: &Observed) -> Vec<(RpcFallbackReason, RpcFallbackOutcome)> {
        observed.lock().unwrap().clone()
    }

    const LOWER: H256 = H256::repeat_byte(0x42);
    const START: u64 = 1_731;
    const END: u64 = 1_760;

    /// What `START..=END` must chain to: every root built by the source, from `LOWER`.
    async fn all_from_source() -> Vec<H256> {
        MockEthRpcProvider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .unwrap()
            .iter()
            .map(Block::digest)
            .collect()
    }

    fn digests(blocks: &[Block]) -> Vec<H256> {
        blocks.iter().map(Block::digest).collect()
    }

    /// Request `START..=END` from an archiver at `url` with a working source RPC and check that
    /// it was served in full from the source RPC because of `reason`.
    async fn assert_whole_range_from_source(url: String, reason: RpcFallbackReason) {
        let source = Arc::new(CountingSource::default());
        let (provider, seen) = provider(url, source.clone());

        let blocks = provider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .unwrap_or_else(|e| panic!("{reason:?} must be covered by the source RPC: {e:#}"));

        assert_eq!(digests(&blocks), all_from_source().await);
        assert_eq!(source.ranges(), vec![(START, END)]);
        assert_eq!(observed(&seen), vec![(reason, RpcFallbackOutcome::Served)]);
    }

    #[tokio::test]
    async fn a_fully_archived_range_never_touches_the_source_rpc() {
        let url = archiver_storing_up_to(END, Some(END)).await;
        let source = Arc::new(CountingSource::default());
        let (provider, seen) = provider(url, source.clone());

        let blocks = provider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .unwrap();

        assert_eq!(digests(&blocks), all_from_source().await);
        assert!(source.ranges().is_empty());
        assert!(observed(&seen).is_empty());
    }

    #[tokio::test]
    async fn catching_up_builds_only_the_missing_tail_from_the_source_rpc() {
        // The cc3-devnet case: 30-block range, archiver 8 blocks short of its end.
        let url = archiver_storing_up_to(1_752, Some(1_752)).await;
        let source = Arc::new(CountingSource::default());
        let (provider, seen) = provider(url, source.clone());

        let blocks = provider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .unwrap();

        assert_eq!(
            digests(&blocks),
            all_from_source().await,
            "archived prefix and source tail must chain into one digest sequence"
        );
        assert_eq!(source.ranges(), vec![(1_753, END)]);
        assert_eq!(
            observed(&seen),
            vec![(RpcFallbackReason::CatchingUp, RpcFallbackOutcome::Served)]
        );
    }

    #[tokio::test]
    async fn catching_up_with_none_of_the_range_stored_builds_it_all_from_the_source_rpc() {
        let url = archiver_storing_up_to(1_720, Some(1_720)).await;
        assert_whole_range_from_source(url, RpcFallbackReason::CatchingUp).await;
    }

    #[tokio::test]
    async fn catching_up_whose_prefix_cannot_be_served_builds_it_all_from_the_source_rpc() {
        // `/roots/latest` claims 1752, but the store cannot serve the prefix up to it.
        let url = archiver_storing_up_to(1_740, Some(1_752)).await;
        assert_whole_range_from_source(url, RpcFallbackReason::CatchingUp).await;
    }

    #[tokio::test]
    async fn a_stored_prefix_past_the_range_end_fetches_only_the_range() {
        // Callers only pass a height below `END` today; the helper must still not over-fetch, or
        // underflow building an empty tail, if one ever passes a higher one.
        let requested = Arc::new(Mutex::new(Vec::new()));
        let seen = requested.clone();
        let url = serve(Arc::new(move |path: &str| -> (u16, String) {
            let (from, to) = range_of(path);
            seen.lock().unwrap().push((from, to));
            (200, roots_body(from..=to))
        }))
        .await;
        let source = Arc::new(CountingSource::default());
        let (provider, _) = provider(url, source.clone());

        let blocks = provider
            .build_with_source_tail(LOWER, START, END, 5_000)
            .await
            .unwrap();

        assert_eq!(digests(&blocks), all_from_source().await);
        assert_eq!(*requested.lock().unwrap(), vec![(START, END)]);
        assert!(source.ranges().is_empty(), "no tail left to build");
    }

    #[tokio::test]
    async fn a_hole_in_the_archive_is_covered_by_the_source_rpc() {
        let url = archiver_storing_up_to(1_700, Some(5_000)).await;
        assert_whole_range_from_source(url, RpcFallbackReason::Hole).await;
    }

    #[tokio::test]
    async fn a_stalled_archiver_is_covered_by_the_source_rpc() {
        // More than one attestation's worth (500 blocks) behind the end of the range.
        let url = archiver_storing_up_to(1_000, Some(1_000)).await;
        assert_whole_range_from_source(url, RpcFallbackReason::Stalled).await;
    }

    #[tokio::test]
    async fn an_incomplete_range_with_no_known_height_is_covered_by_the_source_rpc() {
        let url = archiver_storing_up_to(1_700, None).await;
        assert_whole_range_from_source(url, RpcFallbackReason::UnknownHeight).await;
    }

    #[tokio::test]
    async fn an_archiver_server_error_is_covered_by_the_source_rpc() {
        let url = archiver_answering(500, "store read failed").await;
        assert_whole_range_from_source(url, RpcFallbackReason::ArchiverFault).await;
    }

    #[tokio::test]
    async fn an_unreachable_archiver_is_covered_by_the_source_rpc() {
        let url = unreachable_archiver().await;
        assert_whole_range_from_source(url, RpcFallbackReason::Unreachable).await;
    }

    #[tokio::test]
    async fn a_hung_archiver_times_out_and_is_covered_by_the_source_rpc() {
        let source = Arc::new(CountingSource::default());
        let (mut provider, seen) = provider(hung_archiver().await, source.clone());
        provider.archiver.roots_timeout = Duration::from_millis(200);

        let blocks = tokio::time::timeout(
            Duration::from_secs(10),
            provider.build_continuity_blocks(LOWER, START, END),
        )
        .await
        .expect("the roots deadline cuts the archiver off")
        .unwrap();

        assert_eq!(digests(&blocks), all_from_source().await);
        assert_eq!(source.ranges(), vec![(START, END)]);
        assert_eq!(
            observed(&seen),
            vec![(RpcFallbackReason::Unreachable, RpcFallbackOutcome::Served)]
        );
    }

    #[tokio::test]
    async fn an_unparseable_archiver_response_is_covered_by_the_source_rpc() {
        let url = archiver_answering(200, "<html>bad gateway</html>").await;
        assert_whole_range_from_source(url, RpcFallbackReason::BadResponse).await;
    }

    #[tokio::test]
    async fn out_of_order_archiver_roots_are_covered_by_the_source_rpc() {
        let url = serve(Arc::new(|path: &str| -> (u16, String) {
            let (from, to) = range_of(path);
            (200, roots_body((from..=to).rev()))
        }))
        .await;
        assert_whole_range_from_source(url, RpcFallbackReason::BadResponse).await;
    }

    #[tokio::test]
    async fn a_range_rejection_is_not_covered_by_the_source_rpc() {
        let url = archiver_answering(400, "range too large (max 1000 blocks)").await;
        let source = Arc::new(CountingSource::default());
        let (provider, seen) = provider(url, source.clone());

        let err = provider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .expect_err("a range rejection is passed through");

        assert!(status_error(&err).is_range_rejection());
        assert!(source.ranges().is_empty());
        assert!(observed(&seen).is_empty());
    }

    #[tokio::test]
    async fn a_failed_fallback_answers_with_the_archivers_catching_up_error() {
        let url = archiver_storing_up_to(1_752, Some(1_752)).await;
        let source = Arc::new(CountingSource {
            fail: true,
            ..Default::default()
        });
        let (provider, seen) = provider(url, source.clone());

        let err = provider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .expect_err("both sources failed");

        let status = status_error(&err);
        assert!(
            status.is_catching_up_at_tip(),
            "the caller must still get the retriable catching-up classification"
        );
        assert_eq!(status.latest_archived, Some(1_752));
        assert_eq!(
            observed(&seen),
            vec![(RpcFallbackReason::CatchingUp, RpcFallbackOutcome::Failed)]
        );
    }

    #[tokio::test]
    async fn a_failed_fallback_for_an_archiver_fault_answers_with_the_archivers_error() {
        let url = archiver_answering(503, "store unavailable").await;
        let source = Arc::new(CountingSource {
            fail: true,
            ..Default::default()
        });
        let (provider, seen) = provider(url, source.clone());

        let err = provider
            .build_continuity_blocks(LOWER, START, END)
            .await
            .expect_err("both sources failed");

        assert_eq!(
            status_error(&err).status,
            reqwest::StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            observed(&seen),
            vec![(RpcFallbackReason::ArchiverFault, RpcFallbackOutcome::Failed)]
        );
    }
}
