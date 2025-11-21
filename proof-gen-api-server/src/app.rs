use crate::routes::{continuity, health};
use axum::{routing::get, Router};

pub fn build_app() -> Router {
    Router::new()
        .route("/api/v1/health", get(health::health_check))
        .route(
            "/api/v1/proof/{chain_key}/{header_number}",
            get(continuity::get_continuity_proof),
        )
        .route(
            "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
            get(continuity::get_continuity_proof_with_tx),
        )
        .route(
            "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
            get(continuity::get_proof_by_tx_hash),
        )
}
