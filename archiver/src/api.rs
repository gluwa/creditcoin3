//! HTTP API for serving archived root data.
//!
//! Endpoints:
//! - GET /status          — archiver status (latest block, total stored)
//! - GET /roots/latest    — latest archived block number
//! - GET /roots           — roots for a block range (?from=X&to=Y)

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;
use tower_http::trace::TraceLayer;

use crate::store::RootStore;

/// Shared application state for the API handlers.
pub struct AppState {
    pub store: RootStore,
    pub max_api_range: u64,
    /// Caps the number of in-flight `/roots` requests. The endpoint scans a block
    /// range out of sled, so unbounded concurrency lets a few large fan-out clients
    /// exhaust IO/CPU/memory. We `try_acquire` a permit per request and reject with
    /// HTTP 429 when none are free, rather than queueing unboundedly.
    pub roots_semaphore: Arc<Semaphore>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/roots/latest", get(roots_latest))
        .route("/roots", get(roots))
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

    Json(StatusResponse {
        latest_archived_block: latest,
        total_blocks: state.store.count(),
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
pub struct RootEntry {
    pub block_number: u64,
    pub merkle_root: String,
}

async fn roots(
    State(state): State<Arc<AppState>>,
    Query(params): Query<RangeParams>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Validate cheap query params *before* taking a concurrency permit. Invalid requests
    // fail fast without consuming a slot, so they cannot be used to exhaust
    // `MAX_API_CONCURRENCY` and 429 out legitimate range scans (the permit only needs to
    // guard the expensive sled scan below).
    if params.to < params.from {
        return Err((
            StatusCode::BAD_REQUEST,
            "\"to\" must be >= \"from\"".to_string(),
        ));
    }
    let max_range = state.max_api_range;
    if params.to - params.from >= max_range {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("range too large (max {max_range} blocks)"),
        ));
    }

    // Concurrency guard: acquire a permit for the duration of the expensive store scan.
    // When the semaphore is exhausted, reject immediately with 429 instead of blocking —
    // the permit is released when `_permit` drops at the end of the handler.
    let _permit = state.roots_semaphore.try_acquire().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            "too many concurrent /roots requests, retry later".to_string(),
        )
    })?;

    let range = state
        .store
        .get_range(params.from, params.to)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let expected_count = (params.to - params.from + 1) as usize;
    if range.len() != expected_count {
        return Err((
            StatusCode::NOT_FOUND,
            format!(
                "incomplete data: expected {} roots for range {}..={}, found {}",
                expected_count,
                params.from,
                params.to,
                range.len()
            ),
        ));
    }

    let entries: Vec<RootEntry> = range
        .into_iter()
        .map(|(height, stored)| RootEntry {
            block_number: height,
            merkle_root: format!("{:?}", stored.root),
        })
        .collect();

    Ok(Json(entries))
}
