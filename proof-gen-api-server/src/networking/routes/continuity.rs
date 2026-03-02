use axum::http::StatusCode;
use axum::{extract::Path, Extension, Json};
use std::{
    collections::{BTreeMap, BTreeSet, HashSet},
    sync::Arc,
};

use crate::prom::{GetErrorType, Metrics};
use crate::services::continuity_service::{
    BatchedContinuityResponse, ContinuityResponse, ContinuityService, ProofQuery,
    SingleContinuityResponse,
};
use crate::services::{errors::ErrorResponse, ServiceError};

type ApiResult = Result<Json<ContinuityResponse>, (StatusCode, Json<ErrorResponse>)>;
type ProofQueries = Vec<ProofQuery>;
type TxHashes = Vec<String>;

/// Get continuity proof with merkle proof for a transaction at a specific index.
#[utoipa::path(
    get,
    path = "/api/v1/proof/{chain_key}/{header_number}/{tx_index}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
        ("header_number" = u64, Path, description = "Block number"),
        ("tx_index" = u64, Path, description = "Transaction index in the block (0 for empty blocks)")
    ),
    responses(
        (status = 200, description = "Continuity and merkle proof", body = SingleContinuityResponse),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 404, description = "Block or transaction not found", body = ErrorResponse),
        (status = 503, description = "RPC unavailable", body = ErrorResponse)
    )
)]
pub async fn get_proof_with_tx(
    Path((chain_key, header_number, tx_index)): Path<(u64, u64, u64)>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
) -> ApiResult {
    service
        .get_proof(chain_key, header_number, tx_index)
        .await
        .inspect(|r| {
            tracing::info!(
                chain_key = chain_key,
                header_number = r.header_number,
                tx_index = r.tx_index,
                cached = r.cached,
                "Request served"
            )
        })
        .map(ContinuityResponse::from)
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}

/// Get continuity and merkle proof by transaction hash.
#[utoipa::path(
    get,
    path = "/api/v1/proof-by-tx/{chain_key}/{tx_hash}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
        ("tx_hash" = String, Path, description = "Transaction hash (0x-prefixed hex)")
    ),
    responses(
        (status = 200, description = "Continuity and merkle proof", body = SingleContinuityResponse),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 404, description = "Transaction not found", body = ErrorResponse),
        (status = 501, description = "Tx hash lookup not implemented", body = ErrorResponse)
    )
)]
pub async fn get_proof_by_tx_hash(
    Path((chain_key, tx_hash)): Path<(u64, String)>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
) -> ApiResult {
    service
        .get_proof_by_tx_hash(chain_key, tx_hash.clone())
        .await
        .inspect(|r| tracing::info!(chain_key = chain_key, %tx_hash, header_number = r.header_number, cached = r.cached, "Request served"))
        .map(ContinuityResponse::from)
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}

#[utoipa::path(
    post,
    path = "/api/v1/proof-batch/{chain_key}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
    ),
    request_body(content = inline(ProofQueries), content_type = "application/json", description = "Array of proof queries"),
    responses(
        (status = 200, description = "Continuity and merkle proof entries", body = BatchedContinuityResponse),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 404, description = "Transaction not found", body = ErrorResponse),
        (status = 501, description = "Tx hash lookup not implemented", body = ErrorResponse)
    )
)]
pub async fn get_proof_batch(
    Path(chain_key): Path<u64>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
    Json(proof_queries): Json<ProofQueries>,
) -> ApiResult {
    // We enforce some limits on batch size and tx query size here at the API layer to prevent abuse and excessive load on the service layer.
    if proof_queries.is_empty() {
        let error = ServiceError::EmptyProofQueries;
        metrics.inc_error(error.error_type());
        return Err(error.into_response());
    }
    if proof_queries.len() > 10 {
        let error = ServiceError::TooManyProofQueries;
        metrics.inc_error(error.error_type());
        return Err(error.into_response());
    }
    if proof_queries.iter().any(|q| q.tx_indexes.len() > 10) {
        let error = ServiceError::TooManyTxQueriesInProofQuery;
        metrics.inc_error(error.error_type());
        return Err(error.into_response());
    }

    // We deduplicate proof queries by header number and tx index to avoid redundant work in the service layer.
    // For example, if the batch contains multiple queries for the same header and tx index, we only need to generate one proof for that header and tx index.
    let unique_proof_queries = {
        let mut query_map = BTreeMap::new();

        for query in proof_queries {
            let entry = query_map
                .entry(query.header_number)
                .or_insert(BTreeSet::new());

            for tx_index in query.tx_indexes {
                entry.insert(tx_index);
            }
        }

        query_map
            .into_iter()
            .map(|(header_number, tx_index_set)| ProofQuery {
                header_number,
                tx_indexes: tx_index_set.into_iter().collect(),
            })
            .collect::<Vec<_>>()
    };

    service
        .get_proof_batch(chain_key, &unique_proof_queries)
        .await
        .inspect(|r| {
            tracing::info!(
                chain_key = chain_key,
                from_header = r.from_header,
                to_header = r.to_header,
                cached = r.cached,
                "Batch request served"
            )
        })
        .map(ContinuityResponse::from)
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}

#[utoipa::path(
    post,
    path = "/api/v1/proof-batch-by-tx/{chain_key}",
    params(
        ("chain_key" = u64, Path, description = "Chain identifier"),
    ),
    request_body(content = inline(TxHashes), content_type = "application/json", description = "Array of transaction hashes (0x-prefixed hex)"),
    responses(
        (status = 200, description = "Continuity and merkle proof entries", body = BatchedContinuityResponse),
        (status = 400, description = "Invalid parameters", body = ErrorResponse),
        (status = 404, description = "Transaction not found", body = ErrorResponse),
        (status = 501, description = "Tx hash lookup not implemented", body = ErrorResponse)
    )
)]
pub async fn get_proof_batch_by_tx_hash(
    Path(chain_key): Path<u64>,
    Extension(service): Extension<Arc<ContinuityService>>,
    Extension(metrics): Extension<Metrics>,
    Json(tx_hashes): Json<TxHashes>,
) -> ApiResult {
    // First we validate the request to ensure it contains a reasonable number of tx hashes to prevent abuse and excessive load on the service layer.
    if tx_hashes.is_empty() {
        let error = ServiceError::EmptyTxHashes;
        metrics.inc_error(error.error_type());
        return Err(error.into_response());
    }
    if tx_hashes.len() > 100 {
        let error = ServiceError::TooManyTxHashes;
        metrics.inc_error(error.error_type());
        return Err(error.into_response());
    }

    // Remove duplicate tx hashes to avoid redundant work in the service layer
    let unique_tx_hashes = tx_hashes
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    service
        .get_proof_batch_by_tx_hashes(chain_key, unique_tx_hashes.as_slice())
        .await
        .inspect(|r| {
            tracing::info!(
                chain_key = chain_key,
                from_header = r.from_header,
                to_header = r.to_header,
                cached = r.cached,
                "Batch request served"
            )
        })
        .map(ContinuityResponse::from)
        .map(Json)
        .map_err(|e| {
            metrics.inc_error(e.error_type());
            e.into_response()
        })
}
