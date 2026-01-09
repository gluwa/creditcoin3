use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use std::sync::Arc;

use crate::services::continuity_service::{ContinuityResponse, ContinuityService};
use crate::services::errors::{ErrorResponse, ServiceError};

type ApiResult = Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)>;

impl ServiceError {
    fn status_code(&self) -> StatusCode {
        match self {
            Self::AttestationsMissing { .. } => StatusCode::NOT_FOUND,
            Self::QueryOutOfRange { .. }
            | Self::TxIndexOutOfBounds { .. }
            | Self::InvalidParameter { .. }
            | Self::BlockBeforeGenesis { .. } => StatusCode::BAD_REQUEST,
            Self::RpcUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TxHashLookupUnavailable { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::TxHashNotFound { .. }
            | Self::BlockNotReady { .. }
            | Self::BlockNotOnSourceChain { .. } => StatusCode::NOT_FOUND,
            Self::DbError { .. } | Self::MerkleError { .. } | Self::Internal { .. } => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }
    }

    fn into_response(self) -> (StatusCode, Json<ErrorResponse>) {
        let status = self.status_code();
        let response = ErrorResponse::from_service_error(&self);
        (status, Json(response))
    }
}

pub async fn get_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
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
        .map_err(ServiceError::into_response)
}

pub async fn get_proof_with_tx(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
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
        .map_err(ServiceError::into_response)
}

pub async fn get_proofs_by_tx_hash(
    Path((chain_key, tx_hash)): Path<(u64, String)>,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> ApiResult {
    service
        .get_proofs_by_tx_hash(chain_key, tx_hash.clone())
        .await
        .inspect(|r| tracing::info!(chain_key, %tx_hash, header_number = r.header_number, cached = r.cached, "Request served"))
        .map(Json)
        .map_err(ServiceError::into_response)
}
