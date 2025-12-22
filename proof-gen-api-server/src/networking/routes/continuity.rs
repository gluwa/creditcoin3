use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use std::sync::Arc;

use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use crate::services::errors::{ErrorResponse, ServiceError};

fn map_service_error(err: ServiceError) -> (StatusCode, Json<ErrorResponse>) {
    let status = match &err {
        ServiceError::AttestationsMissing { .. } => StatusCode::NOT_FOUND,
        ServiceError::QueryOutOfRange { .. } => StatusCode::BAD_REQUEST,
        ServiceError::TxIndexOutOfBounds { .. } => StatusCode::BAD_REQUEST,
        ServiceError::InvalidParameter { .. } => StatusCode::BAD_REQUEST,
        ServiceError::RpcUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        ServiceError::TxHashLookupUnavailable { .. } => StatusCode::NOT_IMPLEMENTED,
        ServiceError::TxHashNotFound { .. } => StatusCode::NOT_FOUND,
        ServiceError::DbError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        ServiceError::MerkleError { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        ServiceError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        ServiceError::BlockNotReady { .. } => StatusCode::SERVICE_UNAVAILABLE,
    };
    let response = ErrorResponse::from_service_error(&err);
    (status, Json(response))
}

pub async fn get_continuity_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)> {
    match service.get_continuity_proof(chain_key, header_number).await {
        Ok(resp) => {
            tracing::info!(
                chain_key,
                header_number,
                cached = resp.cached,
                "Request served successfully"
            );
            Ok(Json(resp))
        }
        Err(e) => Err(map_service_error(e)),
    }
}

pub async fn get_proofs_by_height_and_index(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)> {
    match service
        .get_proofs_by_height_and_index(chain_key, header_number, tx_index)
        .await
    {
        Ok(resp) => {
            tracing::info!(
                chain_key,
                header_number,
                tx_index,
                cached = resp.cached,
                "Request served successfully"
            );
            Ok(Json(resp))
        }
        Err(e) => Err(map_service_error(e)),
    }
}

pub async fn get_proofs_by_tx_hash(
    Path((chain_key, tx_hash)): Path<(u64, String)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)> {
    let tx_hash_clone = tx_hash.clone();
    match service.get_proofs_by_tx_hash(chain_key, tx_hash).await {
        Ok(resp) => {
            tracing::info!(
                chain_key,
                tx_hash = %tx_hash_clone,
                header_number = resp.header_number,
                cached = resp.cached,
                "Request served successfully"
            );
            Ok(Json(resp))
        }
        Err(e) => Err(map_service_error(e)),
    }
}
