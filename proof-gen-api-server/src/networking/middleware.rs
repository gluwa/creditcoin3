use axum::{
    body::Body,
    extract::Request,
    http::{StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension,
};
use http_body_util::BodyExt;
use serde_json::json;
use std::time::Instant;

use crate::prom::{Endpoint, Metrics, Status};

/// Middleware that records request metrics (count, duration, and sizes).
pub async fn request_metrics_middleware(
    Extension(metrics): Extension<Metrics>,
    request: Request,
    next: Next,
) -> Response {
    let uri = request.uri().clone();
    let endpoint = extract_endpoint_from_path(&uri);

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

        // Record response size
        // We need to consume and recreate the body to get the size
        let (parts, body) = response.into_parts();
        match body.collect().await {
            Ok(collected) => {
                let bytes = collected.to_bytes();
                let response_size = bytes.len() as u64;
                metrics.observe_response_size(ep, response_size);
                // Rebuild the response with the collected body
                Response::from_parts(parts, Body::from(bytes))
            }
            Err(e) => {
                // If we can't collect the body, log the error and return response with original status
                // Note: Once response.into_parts() is called, the original body is consumed and cannot
                // be recovered. We preserve the status code and headers, but must return an empty body.
                // This scenario is extremely rare for in-memory JSON responses, but if it occurs,
                // the error is logged for debugging.
                tracing::error!(
                    status = %parts.status,
                    "Failed to collect response body for metrics - returning empty body: {e}"
                );
                // Return empty body but preserve the original status code and headers
                Response::from_parts(parts, Body::empty())
            }
        }
    } else {
        response
    }
}

/// Parses API path segments.
/// Returns (endpoint_type, chain_key, parts_count) if the path matches known patterns.
/// - endpoint_type: "proof", "proof-by-tx", "health", or None
/// - chain_key: parsed chain_key or None
/// - parts_count: number of path segments (for determining proof vs proof-with-tx)
fn parse_api_path(path: &str) -> (Option<&str>, Option<u64>, usize) {
    if path == "/api/v1/health" || path.starts_with("/health/") {
        return (Some("health"), None, 0);
    }

    if !path.starts_with("/api/v1/") {
        return (None, None, 0);
    }

    let parts: Vec<&str> = path.split('/').collect();
    // parts[0] = ""
    // parts[1] = "api"
    // parts[2] = "v1"
    // parts[3] = "proof" or "proof-by-tx"
    // parts[4] = chain_key
    // parts[5+] = additional path segments

    if parts.len() < 4 {
        return (None, None, parts.len());
    }

    let endpoint_type = parts[3];
    let chain_key = if parts.len() >= 5 {
        parts[4].parse().ok()
    } else {
        None
    };

    (Some(endpoint_type), chain_key, parts.len())
}

/// Extracts the Endpoint enum variant from a URI path.
/// Returns None for paths that don't match known API endpoints.
fn extract_endpoint_from_path(uri: &Uri) -> Option<Endpoint> {
    let path = uri.path();
    let (endpoint_type, _chain_key, parts_count) = parse_api_path(path);

    match endpoint_type {
        Some("health") => Some(Endpoint::Health),
        Some("proof-by-tx") => Some(Endpoint::ProofByTxHash),
        Some("proof") => {
            // parts_count includes: "", "api", "v1", "proof", "chain_key", "header_number", ["tx_index"]
            // So: 6 parts = proof only, 7 parts = proof with tx
            if parts_count == 6 {
                Some(Endpoint::Proof)
            } else if parts_count == 7 {
                Some(Endpoint::ProofWithTx)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Middleware that validates chain_key from the request path against the configured chain_key.
/// Returns 400 Bad Request if the chain_key doesn't match.
pub async fn chain_key_validator_middleware(
    request: Request,
    next: Next,
    configured_chain_key: u64,
) -> Response {
    let uri = request.uri().clone();

    // Extract chain_key from path
    // Paths are: /api/v1/proof/{chain_key}/{header_number}
    //            /api/v1/proof/{chain_key}/{header_number}/{tx_index}
    //            /api/v1/proof-by-tx/{chain_key}/{tx_hash}
    // Note: extract_chain_key_from_path returns None for health endpoints, so validation
    // automatically skips them without needing an explicit check.
    if let Some(request_chain_key) = extract_chain_key_from_path(&uri) {
        if request_chain_key != configured_chain_key {
            return (
                StatusCode::BAD_REQUEST,
                axum::Json(json!({
                    "code": "InvalidChainKey",
                    "message": format!(
                        "Chain key mismatch: expected {}, got {}",
                        configured_chain_key, request_chain_key
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
fn extract_chain_key_from_path(uri: &Uri) -> Option<u64> {
    let path = uri.path();
    let (endpoint_type, chain_key, _parts_count) = parse_api_path(path);

    // Only extract chain_key for proof endpoints
    if matches!(endpoint_type, Some("proof") | Some("proof-by-tx")) {
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
}
