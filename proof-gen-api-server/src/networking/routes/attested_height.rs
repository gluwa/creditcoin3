use crate::networking::extract::Chain;
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
    responses((status = 200, description = "Height of last finalized attestation in cache", body = AttestedHeightResponse))
)]
pub async fn attested_height(
    chain: Chain,
    Extension(service): Extension<Arc<ContinuityService>>,
) -> Json<AttestedHeightResponse> {
    let chain_key = chain.key;
    let height = service.attested_height(chain_key).await;

    let height = match height {
        Ok(Some(height)) => {
            tracing::debug!(
                chain_key = chain_key,
                attested_height = height,
                "✅ Latest attested height fetched"
            );
            Some(height)
        }
        Ok(None) => {
            tracing::warn!(chain_key = chain_key, "⚠️ No attestations found in cache");
            None
        }
        Err(e) => {
            tracing::error!(
                chain_key = chain_key,
                error = %e,
                "❌ Failed to fetch attested height"
            );
            None
        }
    };

    Json(AttestedHeightResponse {
        attested_height: height,
    })
}
