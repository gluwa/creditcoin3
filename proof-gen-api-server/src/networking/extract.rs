//! Axum extractors for continuity routes.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::{Extension, Json};
use std::ops::Deref;
use std::sync::Arc;

use super::middleware::extract_chain_key_from_path;
use crate::services::continuity_service::{ChainState, ContinuityService};
use crate::services::errors::{ErrorResponse, ServiceError};

/// Resolved chain for a request: `chain_key` from the URL and shared [`ChainState`].
///
/// Use together with path params that include `chain_key`; the extractor resolves state from
/// [`ContinuityService`]. Prefer `chain.key` over repeating the first path segment.
#[derive(Clone)]
pub struct Chain {
    pub key: u64,
    pub state: Arc<ChainState>,
}

impl Deref for Chain {
    type Target = Arc<ChainState>;

    fn deref(&self) -> &Self::Target {
        &self.state
    }
}

impl<S> FromRequestParts<S> for Chain
where
    S: Send + Sync,
{
    type Rejection = (StatusCode, Json<ErrorResponse>);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let Extension(service) =
            Extension::<Arc<ContinuityService>>::from_request_parts(parts, state)
                .await
                .map_err(|_| {
                    (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(ErrorResponse {
                            code: "Internal".to_string(),
                            message: "ContinuityService extension missing".to_string(),
                            retriable: false,
                            block_number: None,
                            last_attested_block: None,
                        }),
                    )
                })?;

        let key = extract_chain_key_from_path(&parts.uri).ok_or_else(|| {
            ServiceError::InvalidParameter {
                message: "could not parse chain_key from request path".to_string(),
            }
            .into_response()
        })?;

        let state = service
            .chain_state(key)
            .map_err(ServiceError::into_response)?
            .clone();

        Ok(Chain { key, state })
    }
}
