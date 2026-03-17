//! HTTP API for serving archived root data to the proof gen API server.
//!
//! Endpoints:
//! - GET /status          — archiver status (latest block, total stored)
//! - GET /roots/latest    — latest archived block number
//! - GET /roots           — roots for a block range (?from=X&to=Y)
//! - GET /proof-input     — ready-made ContinuityProof input (?from=X&to=Y)

use std::sync::Arc;

use anyhow::Context;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use ccnext_abi_encoding::common::EncodingVersion;
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

use crate::store::RootStore;

/// Shared application state for the API handlers.
pub struct AppState {
    pub store: RootStore,
    /// RPC client for fetching blocks not yet in the store (e.g. genesis).
    pub eth_client: eth::Client,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/roots/latest", get(roots_latest))
        .route("/roots", get(roots))
        .route("/proof-input", get(proof_input))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ── Handlers ────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct StatusResponse {
    latest_archived_block: Option<u64>,
    total_blocks: usize,
}

async fn status(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let latest = state.store.latest_height().ok().flatten();
    let first = state.store.first_height().ok().flatten();

    let total = match (first, latest) {
        (Some(f), Some(l)) => (l - f + 1) as usize,
        _ => 0,
    };

    Json(StatusResponse {
        latest_archived_block: latest,
        total_blocks: total,
    })
}

#[derive(Serialize)]
struct LatestResponse {
    latest_block: Option<u64>,
}

async fn roots_latest(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let latest = state.store.latest_height().ok().flatten();
    Json(LatestResponse {
        latest_block: latest,
    })
}

#[derive(Deserialize)]
struct RangeParams {
    from: u64,
    to: u64,
}

#[derive(Serialize)]
struct RootEntry {
    block_number: u64,
    merkle_root: String,
}

async fn roots(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RangeParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if params.to < params.from {
        return Err((
            StatusCode::BAD_REQUEST,
            "\"to\" must be >= \"from\"".to_string(),
        ));
    }
    if params.to - params.from >= 100_000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "range too large (max 100,000 blocks)".to_string(),
        ));
    }

    let range = state
        .store
        .get_range(params.from, params.to)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let entries: Vec<RootEntry> = range
        .into_iter()
        .map(|(height, root)| RootEntry {
            block_number: height,
            merkle_root: format!("{root:?}"),
        })
        .collect();

    Ok(Json(entries))
}

#[derive(Serialize)]
struct ProofInputResponse {
    lower_endpoint_digest: String,
    roots: Vec<RootEntry>,
}

async fn proof_input(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RangeParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if params.to < params.from {
        return Err((
            StatusCode::BAD_REQUEST,
            "\"to\" must be >= \"from\"".to_string(),
        ));
    }
    if params.from == 0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "\"from\" must be >= 1 (need block from-1 for lower_endpoint_digest)".to_string(),
        ));
    }
    if params.to - params.from >= 100_000 {
        return Err((
            StatusCode::BAD_REQUEST,
            "range too large (max 100,000 blocks)".to_string(),
        ));
    }

    let range = state
        .store
        .get_range(params.from, params.to)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if range.is_empty() {
        return Err((
            StatusCode::NOT_FOUND,
            "no roots found for the requested range".to_string(),
        ));
    }

    let lower_endpoint_digest = compute_digest_at(&state.store, &state.eth_client, params.from - 1)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let roots: Vec<RootEntry> = range
        .into_iter()
        .map(|(height, root)| RootEntry {
            block_number: height,
            merkle_root: format!("{root:?}"),
        })
        .collect();

    Ok(Json(ProofInputResponse {
        lower_endpoint_digest: format!("{lower_endpoint_digest:?}"),
        roots,
    }))
}

/// Interval at which digests are cached (every N blocks).
const DIGEST_CACHE_INTERVAL: u64 = 10_000;

/// Compute the chained digest at a given block height.
///
/// digest(n) = keccak256(n || root(n) || digest(n-1))
///
/// Uses a persistent digest cache to avoid replaying from genesis on every
/// request.
async fn compute_digest_at(
    store: &RootStore,
    eth_client: &eth::Client,
    target_height: u64,
) -> anyhow::Result<sp_core::H256> {
    let first = store.first_height()?.context("store is empty")?;

    let (replay_from, mut digest) = if let Some((cached_h, cached_digest)) =
        store.get_cached_digest_at_or_below(target_height)?
    {
        tracing::debug!(cached_height = cached_h, "using cached digest");
        (cached_h + 1, cached_digest)
    } else {
        (0, sp_core::H256::zero())
    };

    // Phase 1: if we need blocks before the store, fetch from chain.
    if replay_from < first {
        let fetch_to = target_height.min(first.saturating_sub(1));
        if fetch_to >= replay_from {
            tracing::info!(
                from = replay_from,
                to = fetch_to,
                "fetching {} blocks from chain for digest computation",
                fetch_to - replay_from + 1
            );
            for height in replay_from..=fetch_to {
                let root = fetch_root_from_chain(eth_client, height).await?;
                digest = attestor_primitives::compute_digest_for(height, &root, Some(&digest));

                if height % DIGEST_CACHE_INTERVAL == 0 {
                    store.cache_digest(height, digest)?;
                }
            }
        }

        if target_height < first {
            if target_height > 0 && target_height % DIGEST_CACHE_INTERVAL == 0 {
                store.cache_digest(target_height, digest)?;
            }
            return Ok(digest);
        }
    }

    // Phase 2: replay from store.
    let store_from = replay_from.max(first);
    let roots = store.get_range(store_from, target_height)?;

    let expected_count = if store_from <= target_height {
        (target_height - store_from + 1) as usize
    } else {
        0
    };
    anyhow::ensure!(
        roots.len() == expected_count,
        "store has gaps between blocks {} and {}: expected {} roots, found {}",
        store_from,
        target_height,
        expected_count,
        roots.len(),
    );

    for (height, root) in &roots {
        digest = attestor_primitives::compute_digest_for(*height, root, Some(&digest));

        if *height % DIGEST_CACHE_INTERVAL == 0 {
            store.cache_digest(*height, digest)?;
        }
    }

    Ok(digest)
}

/// Fetch a single block from the chain and compute its merkle root.
async fn fetch_root_from_chain(
    eth_client: &eth::Client,
    height: u64,
) -> anyhow::Result<sp_core::H256> {
    use user::prelude::*;

    let block = eth_client
        .get_block(height, EncodingVersion::V1)
        .await
        .map_err(|e| match e {
            Interrupt::Cont(err) => anyhow::anyhow!("failed to fetch block {height}: {err}"),
            Interrupt::Stop => anyhow::anyhow!("fetch block {height} interrupted"),
        })?;

    Ok(eth::simple_merkle_tree(&block).root())
}
