use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use std::sync::Arc;

use crate::prom::{GetErrorType, Metrics};
use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use crate::services::errors::ErrorResponse;

type ApiResult = Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)>;

pub async fn get_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
) -> ApiResult {
    service
        .get_proof(chain_key, header_number, None)
        .await
        .inspect(|r| {
            tracing::info!(
                chain_key,
                header_number,
                cached = r.cached,
                "Request served"
            )
        })
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}

pub async fn get_proof_with_tx(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
) -> ApiResult {
    service
        .get_proof(chain_key, header_number, Some(tx_index))
        .await
        .inspect(|r| {
            tracing::info!(
                chain_key,
                header_number,
                tx_index,
                cached = r.cached,
                "Request served"
            )
        })
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}

pub async fn get_proofs_by_tx_hash(
    Path((chain_key, tx_hash)): Path<(u64, String)>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
) -> ApiResult {
    service
        .get_proofs_by_tx_hash(chain_key, tx_hash.clone())
        .await
        .inspect(|r| tracing::info!(chain_key, %tx_hash, header_number = r.header_number, cached = r.cached, "Request served"))
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}
