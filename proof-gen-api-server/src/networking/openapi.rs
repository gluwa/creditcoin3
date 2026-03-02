//! OpenAPI specification and Swagger UI configuration.

use utoipa::OpenApi;
use utoipa_swagger_ui::Config;

use crate::services::continuity_service::{
    ContinuityProofSchema, ContinuityResponse, MerkleProofEntrySchema, ProofQuery,
    TransactionMerkleProofSchema,
};
use crate::services::errors::ErrorResponse;

use super::routes::{continuity, health};

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Proof Gen API Server",
        version = "1.0",
        description = "API for on-demand continuity and merkle proof generation for Creditcoin Oracle queries"
    ),
    paths(
        health::health_check,
        continuity::get_proof_with_tx,
        continuity::get_proof_by_tx_hash,
        continuity::get_proof_batch,
        continuity::get_proof_batch_by_tx_hash,
    ),
    components(schemas(
        ContinuityResponse,
        ContinuityProofSchema,
        TransactionMerkleProofSchema,
        MerkleProofEntrySchema,
        ErrorResponse,
        ProofQuery,
        health::HealthCheckResponse,
    ))
)]
pub struct ApiDoc;

/// Swagger UI configuration for serving at /api/swagger.
/// Configures the OpenAPI JSON URL for nested path serving.
pub fn swagger_config() -> Config<'static> {
    Config::from("/api/swagger/openapi.json")
}
