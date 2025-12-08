use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use crate::services::errors::ServiceError;

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
        ServiceError::TxHashLookupUnavailable { .. } => {
            (StatusCode::NOT_IMPLEMENTED, err.code(), err.retriable())
        }
        ServiceError::TxHashNotFound { .. } => (StatusCode::NOT_FOUND, err.code(), err.retriable()),
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
        ServiceError::BlockNotReady { .. } => {
            (StatusCode::SERVICE_UNAVAILABLE, err.code(), err.retriable())
        }
    };
    let mut response = json!({
        "code": code,
        "message": err.to_string(),
        "retriable": retriable
    });

    // Add additional fields for BlockNotReady errors
    if let ServiceError::BlockNotReady {
        block_number,
        current_block,
    } = &err
    {
        response["block_number"] = json!(*block_number);
        response["current_block"] = json!(*current_block);
    }

    (status, Json(response))
}

pub async fn get_continuity_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
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
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
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
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
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
