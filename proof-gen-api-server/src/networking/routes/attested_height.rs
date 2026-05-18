use crate::networking::extract::Chain;
use crate::services::errors::ErrorResponse;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use serde::Serialize;
use std::sync::Arc;
use utoipa::ToSchema;

use crate::services::continuity_service::ContinuityService;

/// Health check response schema for OpenAPI
#[derive(Serialize, ToSchema)]
pub struct AttestedHeightResponse {
    attested_height: Option<u64>,
}

/// Queries the last attested height in the proof gen server
/// Used by USC SDK `waitUntilHeightAttested` to coordinate
/// proof request timing. (Independently having each service
/// listen for new finalized attestations was resulting in
/// timing deltas and failed proving jobs.)
#[utoipa::path(
    get,
    path = "/api/v1/attested-height/{chain_key}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
    ),
    responses(
        (status = 200, description = "Height of last finalized attestation in cache", body = AttestedHeightResponse),
        (status = 500, description = "Error retrieving last attestation from cache", body = ErrorResponse)

    )
)]
pub async fn attested_height(
    chain: Chain,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> impl IntoResponse {
    let chain_key = chain.key;

    match service.attested_height(chain_key).await {
        Ok(Some(height)) => {
            tracing::debug!(
                chain_key = chain_key,
                attested_height = height,
                "✅ Latest attested height fetched"
            );

            (
                StatusCode::OK,
                Json(AttestedHeightResponse {
                    attested_height: Some(height),
                }),
            )
                .into_response()
        }

        Ok(None) => {
            tracing::warn!(chain_key = chain_key, "⚠️ No attestations found in cache");

            (
                StatusCode::OK,
                Json(AttestedHeightResponse {
                    attested_height: None,
                }),
            )
                .into_response()
        }

        Err(e) => {
            tracing::error!(
                chain_key = chain_key,
                error = %e,
                "❌ Failed to fetch attested height"
            );

            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    code: "Internal".to_string(),
                    message: format!(
                        "Failed to fetch attested height for chain key {chain_key}: {e}"
                    ),
                    retriable: true,
                    block_number: None,
                    last_attested_block: None,
                }),
            )
                .into_response()
        }
    }
}
