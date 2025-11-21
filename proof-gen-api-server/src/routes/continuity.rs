use axum::{extract::Path, Json};
use serde_json::{json, Value};

use crate::services::continuity_service;

pub async fn get_continuity_proof(
    Path((chain_key, header_number)): Path<(u64, u64)>,
) -> Json<Value> {
    let proof = continuity_service::get_mock_continuity_proof(chain_key, header_number);

    Json(json!(proof))
}

pub async fn get_continuity_proof_with_tx(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
) -> Json<Value> {
    let proof =
        continuity_service::get_mock_continuity_proof_with_tx(chain_key, header_number, tx_index);

    Json(json!(proof))
}

pub async fn get_proof_by_tx_hash(Path((chain_key, tx_hash)): Path<(u64, String)>) -> Json<Value> {
    let proof = continuity_service::get_mock_proof_by_tx_hash(chain_key, tx_hash);

    Json(json!(proof))
}
