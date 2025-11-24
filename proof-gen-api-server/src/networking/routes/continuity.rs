use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use crate::services::errors::ServiceError;
use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

fn map_service_error(err: ServiceError) -> (StatusCode, Json<Value>) {
    let (status, code, retriable) = match &err {
        ServiceError::AttestationsMissing { .. } => {
            (StatusCode::NOT_FOUND, err.code(), err.retriable())
        }
        ServiceError::QueryOutOfRange { .. } => {
            (StatusCode::BAD_REQUEST, err.code(), err.retriable())
        }
        ServiceError::TxIndexOutOfBounds { .. } => {
            (StatusCode::BAD_REQUEST, err.code(), err.retriable())
        }
        ServiceError::InvalidParameter { .. } => {
            (StatusCode::BAD_REQUEST, err.code(), err.retriable())
        }
        ServiceError::RpcUnavailable { .. } => {
            (StatusCode::SERVICE_UNAVAILABLE, err.code(), err.retriable())
        }
        ServiceError::DbError { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            err.code(),
            err.retriable(),
        ),
        ServiceError::MerkleError { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            err.code(),
            err.retriable(),
        ),
        ServiceError::Internal { .. } => (
            StatusCode::INTERNAL_SERVER_ERROR,
            err.code(),
            err.retriable(),
        ),
    };
    (
        status,
        Json(json!({
            "error": code,
            "message": err.to_string(),
            "retriable": retriable
        })),
    )
}

pub async fn get_continuity_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
    match service.continuity_proof(chain_key, header_number).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Err(map_service_error(e)),
    }
}

pub async fn get_continuity_proof_with_tx(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
    match service
        .continuity_proof_with_tx(chain_key, header_number, tx_index)
        .await
    {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Err(map_service_error(e)),
    }
}

pub async fn get_proof_by_tx_hash(
    Path((chain_key, tx_hash)): Path<(u64, String)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
    match service
        .continuity_proof_by_tx_hash(chain_key, tx_hash)
        .await
    {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Err(map_service_error(e)),
    }
}
