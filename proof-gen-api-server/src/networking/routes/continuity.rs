use crate::networking::ApiError;
use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use serde_json::{json, Value};
use std::sync::Arc;

fn map_anyhow_to_api_error(e: anyhow::Error) -> ApiError {
    let msg = format!("{}", e);
    if msg.contains("No attestations") {
        ApiError::NotFound(msg)
    } else if msg.contains("Invalid request") || msg.contains("invalid") {
        ApiError::BadRequest(msg)
    } else {
        ApiError::InternalError
    }
}

fn api_error_to_response(err: ApiError) -> (StatusCode, Json<Value>) {
    match err {
        ApiError::BadRequest(m) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "BadRequest", "message": m })),
        ),
        ApiError::NotFound(m) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "NotFound", "message": m })),
        ),
        ApiError::InternalError => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "Internal", "message": "internal server error" })),
        ),
    }
}

pub async fn get_continuity_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Result<Json<ContinuityResponse>, (StatusCode, Json<Value>)> {
    match service.continuity_proof(chain_key, header_number).await {
        Ok(resp) => Ok(Json(resp)),
        Err(e) => Err(api_error_to_response(map_anyhow_to_api_error(e))),
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
        Err(e) => Err(api_error_to_response(map_anyhow_to_api_error(e))),
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
        Err(e) => Err(api_error_to_response(map_anyhow_to_api_error(e))),
    }
}
