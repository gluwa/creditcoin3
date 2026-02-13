use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use std::sync::Arc;

use crate::prom::{GetErrorType, Metrics};
use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use crate::services::errors::ErrorResponse;

type ApiResult = Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)>;

/// Get continuity proof with merkle proof for a transaction at a specific index.
#[utoipa::path(
    get,
    path = "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
        ("header_number" = u64, Path, description = "Block number"),
        ("tx_index" = u64, Path, description = "Transaction index in the block (0 for empty blocks)")
    ),
    responses(
        (status = 200, description = "Continuity and merkle proof", body = ContinuityResponse),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 404, description = "Block or transaction not found", body = ErrorResponse),
        (status = 503, description = "RPC unavailable", body = ErrorResponse)
    )
)]
pub async fn get_proof_with_tx(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
) -> ApiResult {
    service
        .get_proof(chain_key, header_number, tx_index)
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

/// Get continuity and merkle proof by transaction hash.
#[utoipa::path(
    get,
    path = "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
        ("tx_hash" = String, Path, description = "Transaction hash (0x-prefixed hex)")
    ),
    responses(
        (status = 200, description = "Continuity and merkle proof", body = ContinuityResponse),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 404, description = "Transaction not found", body = ErrorResponse),
        (status = 501, description = "Tx hash lookup not implemented", body = ErrorResponse)
    )
)]
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
