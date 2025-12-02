use axum::{
    extract::Request,
    http::{StatusCode, Uri},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde_json::json;

/// Middleware that validates chain_key from the request path against the configured chain_key.
/// Returns 400 Bad Request if the chain_key doesn't match.
pub async fn chain_key_validator_middleware(
    request: Request,
    next: Next,
    configured_chain_key: u64,
) -> Response {
    let uri = request.uri().clone();

    // Skip validation for health check endpoint
    if uri.path() == "/api/v1/health" {
        return next.run(request).await;
    }

    // Extract chain_key from path
    // Paths are: /api/v1/proof/{chain_key}/{header_number}
    //            /api/v1/proof/{chain_key}/{header_number}/{tx_index}
    //            /api/v1/proof-by-tx/{chain_key}/{tx_hash}
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

    // Patterns:
    // /api/v1/proof/{chain_key}/{header_number}
    // /api/v1/proof/{chain_key}/{header_number}/{tx_index}
    // /api/v1/proof-by-tx/{chain_key}/{tx_hash}
    if path.starts_with("/api/v1/proof") {
        let parts: Vec<&str> = path.split('/').collect();
        // parts[0] = ""
        // parts[1] = "api"
        // parts[2] = "v1"
        // parts[3] = "proof" or "proof-by-tx"
        // parts[4] = chain_key
        if parts.len() >= 5 {
            return parts[4].parse().ok();
        }
    }

    None
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
