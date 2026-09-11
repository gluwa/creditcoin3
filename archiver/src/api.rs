//! HTTP API for serving archived root data.
//!
//! Endpoints:
//! - GET /status          — liveness + freshness snapshot (always 200 when the store is readable)
//! - GET /ready           — same body; 200 only when the archive is current and durable, else 503
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
use tower_http::trace::TraceLayer;

use crate::health::{Health, HealthSnapshot};
use crate::store::RootStore;

/// Shared application state for the API handlers.
pub struct AppState {
    pub store: RootStore,
    pub max_api_range: u64,
    pub health: Arc<Health>,
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/ready", get(ready))
        .route("/roots/latest", get(roots_latest))
        .route("/roots", get(roots))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

// ── Handlers ────────────────────────────────────────────────────────────────

fn snapshot(state: &AppState) -> Result<HealthSnapshot, (StatusCode, String)> {
    // A storage error is a failure, not `null`: it must page, not read as an empty archive.
    let latest = state.store.latest_height().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("store read failed: {e}"),
        )
    })?;
    Ok(state.health.snapshot(latest, state.store.count()))
}

/// Liveness plus the freshness fields. Always 200 while the store is readable, so a stale
/// archive is visible in the body (`ready`, `lag_blocks`, `source_head_age_ms`, ...) rather
/// than hidden behind a status code.
async fn status(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    snapshot(&state).map(Json)
}

/// Readiness: 200 only when the source head was observed recently, the archive is within the
/// allowed lag of the mature target, and the last durability flush succeeded. Point
/// Kubernetes readiness probes and the proof provider's health check here.
async fn ready(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let snap = snapshot(&state)?;
    let code = if snap.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    Ok((code, Json(snap)))
}

#[derive(Serialize)]
struct LatestResponse {
    latest_block: Option<u64>,
}

async fn roots_latest(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let latest = state.store.latest_height().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("store read failed: {e}"),
        )
    })?;
    Ok(Json(LatestResponse {
        latest_block: latest,
    }))
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
