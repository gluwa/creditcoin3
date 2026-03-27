use axum::{
    extract::{MatchedPath, Request},
    http::{StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crate::prom::{Endpoint, Metrics, Status};

/// Middleware that records request metrics (count, duration, and sizes).
pub async fn request_metrics_middleware(
    Extension(metrics): Extension<Metrics>,
    request: Request,
    next: Next,
) -> Response {
    // Use MatchedPath if available (more reliable), otherwise fall back to parsing URI
    let endpoint = if let Some(matched_path) = request.extensions().get::<MatchedPath>() {
        extract_endpoint_from_matched_path(matched_path.as_str())
    } else {
        // Fallback for cases where MatchedPath isn't available (e.g., nested routers)
        extract_endpoint_from_path(request.uri())
    };

    // Record request size (for GET requests, this is typically small/zero)
    let request_size = request
        .headers()
        .get(axum::http::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    if let Some(ep) = endpoint.clone() {
        metrics.observe_request_size(ep, request_size);
    }

    // Start timing
    let start = Instant::now();

    // Run the handler
    let response = next.run(request).await;
    let duration = start.elapsed();

    // Determine status category from response status code
    let status_code = response.status();
    let status = if status_code.is_success() {
        Status::Success
    } else if status_code.is_client_error() {
        Status::ClientError
    } else {
        Status::ServerError
    };

    // Record metrics if we have a valid endpoint
    if let Some(ep) = endpoint {
        metrics.inc_request(ep.clone(), status);
        metrics.observe_request_duration(ep.clone(), duration);

        // Use Content-Length header when present (axum's Json sets it) to avoid
        // buffering the full body and risking silent data loss on collect failure
        let response_size = response
            .headers()
            .get(axum::http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        metrics.observe_response_size(ep, response_size);
    }

    response
}

/// Parses API path segments.
/// Returns (endpoint_type, chain_key, parts_count) if the path matches known patterns.
/// - endpoint_type: "proof", "proof-by-tx", "health", or None
/// - chain_key: parsed chain_key or None
/// - parts_count: number of path segments (for determining proof vs proof-with-tx)
fn parse_api_path(path: &str) -> (Option<&str>, Option<u64>, usize) {
    if path == "/api/v1/health" {
        return (Some("health"), None, 0);
    }

    if !path.starts_with("/api/v1/") {
        return (None, None, 0);
    }

    // Split path and filter out empty strings (from leading/trailing slashes)
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    // parts[0] = "api"
    // parts[1] = "v1"
    // parts[2] = "proof" or "proof-by-tx" or "proof-batch" or "proof-batch-by-tx"
    // parts[3] = chain_key
    // parts[4+] = additional path segments

    if parts.len() < 3 {
        return (None, None, parts.len());
    }

    let endpoint_type = parts[2];
    let chain_key = if parts.len() >= 4 {
        parts[3].parse().ok()
    } else {
        None
    };

    (Some(endpoint_type), chain_key, parts.len())
}

/// Extracts the Endpoint enum variant from a matched route pattern.
/// This is more reliable than parsing the actual URI since it uses the route definition.
fn extract_endpoint_from_matched_path(matched_path: &str) -> Option<Endpoint> {
    match matched_path {
        "/api/v1/health" => Some(Endpoint::Health),
        "/api/v1/proof/{chain_key}/{header_number}/{tx_index}" => Some(Endpoint::ProofWithTx),
        "/api/v1/proof-by-tx/{chain_key}/{tx_hash}" => Some(Endpoint::ProofByTxHash),
        "/api/v1/proof-batch/{chain_key}" => Some(Endpoint::ProofBatch),
        "/api/v1/proof-batch-by-tx/{chain_key}" => Some(Endpoint::ProofBatchByTxHash),
        "/metrics" => None, // Metrics endpoint doesn't need endpoint classification
        _ => None,
    }
}

/// Extracts the Endpoint enum variant from a URI path (fallback when MatchedPath unavailable).
/// Returns None for paths that don't match known API endpoints.
fn extract_endpoint_from_path(uri: &Uri) -> Option<Endpoint> {
    let path = uri.path();
    let (endpoint_type, _chain_key, parts_count) = parse_api_path(path);

    match endpoint_type {
        Some("health") => Some(Endpoint::Health),
        Some("proof-by-tx") => Some(Endpoint::ProofByTxHash),
        Some("proof-batch-by-tx") => Some(Endpoint::ProofBatchByTxHash),
        Some("proof") => {
            // parts_count includes: "api", "v1", "proof", "chain_key", "header_number", "tx_index"
            // So: 6 parts = proof with tx
            if parts_count == 6 {
                Some(Endpoint::ProofWithTx)
            } else {
                None
            }
        }
        Some("proof-batch") => {
            // parts_count includes: "api", "v1", "proof-batch", "chain_key"
            if parts_count == 4 {
                Some(Endpoint::ProofBatch)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Middleware that validates chain_key from the request path against configured chain keys.
/// Returns 400 Bad Request if the chain_key is not served by this server.
pub async fn chain_key_validator_middleware(
    request: Request,
    next: Next,
    allowed_chain_keys: Arc<HashSet<u64>>,
) -> Response {
    let uri = request.uri().clone();

    // Extract chain_key from path
    // Paths are: /api/v1/proof/{chain_key}/{header_number}/{tx_index}
    //            /api/v1/proof-by-tx/{chain_key}/{tx_hash}
    // Note: extract_chain_key_from_path returns None for health endpoints, so validation
    // automatically skips them without needing an explicit check.
    if let Some(request_chain_key) = extract_chain_key_from_path(&uri) {
        if !allowed_chain_keys.contains(&request_chain_key) {
            let mut allowed: Vec<u64> = allowed_chain_keys.iter().copied().collect();
            allowed.sort_unstable();
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "code": "InvalidChainKey",
                    "message": format!(
                        "Chain key not configured: {} (allowed: {:?})",
                        request_chain_key,
                        allowed
                    ),
                    "retriable": false
                })),
            )
                .into_response();
        }
    }

    next.run(request).await
}

/// Extracts chain_key from API paths.
/// Returns None if the path doesn't match expected patterns or chain_key can't be parsed.
pub(crate) fn extract_chain_key_from_path(uri: &Uri) -> Option<u64> {
    let path = uri.path();
    let (endpoint_type, chain_key, _parts_count) = parse_api_path(path);

    // Only extract chain_key for proof endpoints
    if matches!(
        endpoint_type,
        Some("proof") | Some("proof-by-tx") | Some("proof-batch") | Some("proof-batch-by-tx")
    ) {
        chain_key
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_chain_key_from_path() {
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/proof/2/100".parse().unwrap()),
            Some(2)
        );
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/proof/42/100/5".parse().unwrap()),
            Some(42)
        );
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/proof-by-tx/123/0xabcdef".parse().unwrap()),
            Some(123)
        );
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/proof-batch/7".parse().unwrap()),
            Some(7)
        );
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/proof-batch-by-tx/8".parse().unwrap()),
            Some(8)
        );
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/health".parse().unwrap()),
            None
        );
        assert_eq!(
            extract_chain_key_from_path(&"/api/v1/proof".parse().unwrap()),
            None
        );
        assert_eq!(
            extract_chain_key_from_path(&"/invalid/path".parse().unwrap()),
            None
        );
    }

    #[test]
    fn test_extract_endpoint_from_path_trailing_slash() {
        // Test that trailing slashes don't cause misclassification
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof/2/100/5".parse().unwrap()),
            Some(Endpoint::ProofWithTx)
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof/2/100/5/".parse().unwrap()),
            Some(Endpoint::ProofWithTx) // Should still be ProofWithTx
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof-by-tx/123/0xabcdef".parse().unwrap()),
            Some(Endpoint::ProofByTxHash)
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof-by-tx/123/0xabcdef/".parse().unwrap()),
            Some(Endpoint::ProofByTxHash) // Should still be ProofByTxHash
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof-batch/7".parse().unwrap()),
            Some(Endpoint::ProofBatch)
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof-batch/7/".parse().unwrap()),
            Some(Endpoint::ProofBatch) // Should still be ProofBatch
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof-batch-by-tx/8".parse().unwrap()),
            Some(Endpoint::ProofBatchByTxHash)
        );
        assert_eq!(
            extract_endpoint_from_path(&"/api/v1/proof-batch-by-tx/8/".parse().unwrap()),
            Some(Endpoint::ProofBatchByTxHash) // Should still be ProofBatchByTxHash
        );
    }
}
