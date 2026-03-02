use axum::http::StatusCode;
use axum::Json;
use continuity::ContinuityError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::prom::{ErrorType, GetErrorType};

/// HTTP error response structure returned by the API.
/// This struct is used for both serialization (API responses) and deserialization (tests).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, utoipa::ToSchema)]
pub struct ErrorResponse {
    /// Error code (e.g., "BlockNotReady", "Internal")
    pub code: String,
    /// Human-readable error message
    pub message: String,
    /// Whether the client should retry this request
    pub retriable: bool,
    /// Optional: block number for BlockNotReady errors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    /// Optional: last attested block for BlockNotReady errors
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_attested_block: Option<u64>,
}

impl ErrorResponse {
    /// Create an ErrorResponse from a ServiceError
    pub fn from_service_error(err: &ServiceError) -> Self {
        let code = err.code().to_string();
        let message = err.to_string();
        let retriable = err.retriable();

        let (block_number, last_attested_block) = if let ServiceError::BlockNotReady {
            block_number,
            last_attested_block,
        } = err
        {
            (Some(*block_number), Some(*last_attested_block))
        } else {
            (None, None)
        };

        Self {
            code,
            message,
            retriable,
            block_number,
            last_attested_block,
        }
    }
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("attestations missing for chain {chain_key}")]
    AttestationsMissing { chain_key: u64 },
    #[error("query height {height} out of range")]
    QueryOutOfRange { height: u64 },
    #[error("tx_index {tx_index} at height {height} out of bounds (len={len})")]
    TxIndexOutOfBounds {
        height: u64,
        tx_index: u64,
        len: usize,
    },
    #[error("rpc unavailable: {message}")]
    RpcUnavailable { message: String },
    #[error("merkle proof generation failed: {message}")]
    MerkleError { message: String },
    #[error("invalid parameter: {message}")]
    InvalidParameter { message: String },
    #[error("internal error: {message}")]
    Internal { message: String },
    #[error("tx hash reverse lookup unavailable for {tx_hash}")]
    TxHashLookupUnavailable { tx_hash: String },
    #[error("tx hash not found: {tx_hash}")]
    TxHashNotFound { tx_hash: String },
    #[error("The continuity proof cannot be created because block {block_number} is not attested to yet. Last attested block: {last_attested_block}")]
    BlockNotReady {
        block_number: u64,
        last_attested_block: u64,
    },
    #[error("Block {requested_block} is before attestation genesis block {genesis_block}. Cannot generate proofs for blocks before the attestation system was initialized.")]
    BlockBeforeGenesis {
        requested_block: u64,
        genesis_block: u64,
    },
    #[error("Block {requested_block} does not exist on the source chain yet. Current source chain height: {current_block}")]
    BlockNotOnSourceChain {
        requested_block: u64,
        current_block: u64,
    },
    #[error("Batch request should contain at least one proof query")]
    EmptyProofQueries,
    #[error("Batch request cannot contain more than 10 proof queries")]
    TooManyProofQueries,
    #[error("Each proof query can contain at most 10 tx indexes")]
    TooManyTxQueriesInProofQuery,
    #[error("Batch request should contain at least one tx hash")]
    EmptyTxHashes,
    #[error("Batch request cannot contain more than 100 tx hashes")]
    TooManyTxHashes,
}

impl ServiceError {
    pub fn retriable(&self) -> bool {
        matches!(
            self,
            ServiceError::RpcUnavailable { .. }
                | ServiceError::BlockNotReady { .. }
                | ServiceError::BlockNotOnSourceChain { .. }
        )
    }
    pub fn code(&self) -> &'static str {
        match self {
            ServiceError::AttestationsMissing { .. } => "AttestationsMissing",
            ServiceError::QueryOutOfRange { .. } => "QueryOutOfRange",
            ServiceError::TxIndexOutOfBounds { .. } => "TxIndexOutOfBounds",
            ServiceError::RpcUnavailable { .. } => "RpcUnavailable",
            ServiceError::MerkleError { .. } => "MerkleError",
            ServiceError::InvalidParameter { .. } => "InvalidParameter",
            ServiceError::Internal { .. } => "Internal",
            ServiceError::TxHashLookupUnavailable { .. } => "TxHashLookupUnavailable",
            ServiceError::TxHashNotFound { .. } => "TxHashNotFound",
            ServiceError::BlockNotReady { .. } => "BlockNotReady",
            ServiceError::BlockBeforeGenesis { .. } => "BlockBeforeGenesis",
            ServiceError::BlockNotOnSourceChain { .. } => "BlockNotOnSourceChain",
            ServiceError::EmptyProofQueries => "EmptyProofQueries",
            ServiceError::TooManyProofQueries => "TooManyProofQueries",
            ServiceError::TooManyTxQueriesInProofQuery => "TooManyTxQueriesInProofQuery",
            ServiceError::TooManyTxHashes => "TooManyTxHashes",
            ServiceError::EmptyTxHashes => "EmptyTxHashes",
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::AttestationsMissing { .. } => StatusCode::NOT_FOUND,
            Self::QueryOutOfRange { .. }
            | Self::TxIndexOutOfBounds { .. }
            | Self::InvalidParameter { .. }
            | Self::BlockBeforeGenesis { .. }
            | Self::EmptyProofQueries
            | Self::TooManyProofQueries
            | Self::TooManyTxQueriesInProofQuery
            | Self::EmptyTxHashes
            | Self::TooManyTxHashes => StatusCode::BAD_REQUEST,
            Self::RpcUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TxHashLookupUnavailable { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::TxHashNotFound { .. }
            | Self::BlockNotReady { .. }
            | Self::BlockNotOnSourceChain { .. } => StatusCode::NOT_FOUND,
            Self::MerkleError { .. } | Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn into_response(self) -> (StatusCode, Json<ErrorResponse>) {
        let status = self.status_code();
        let response = ErrorResponse::from_service_error(&self);
        (status, Json(response))
    }
}

impl From<ContinuityError> for ServiceError {
    fn from(e: ContinuityError) -> Self {
        match e {
            ContinuityError::BlockNotReady {
                block_number,
                last_attested_block,
            } => ServiceError::BlockNotReady {
                block_number,
                last_attested_block,
            },
            ContinuityError::NoAttestations(chain_key) => {
                ServiceError::AttestationsMissing { chain_key }
            }
            ContinuityError::Rpc(msg) => ServiceError::RpcUnavailable { message: msg },
            ContinuityError::Internal(msg) => ServiceError::Internal { message: msg },
            ContinuityError::InvalidBounds(msg) => ServiceError::InvalidParameter { message: msg },
            ContinuityError::MissingBlock => ServiceError::Internal {
                message: "Block not found in continuity chain".to_string(),
            },
            ContinuityError::BlockBeforeGenesis {
                requested_block,
                genesis_block,
            } => ServiceError::BlockBeforeGenesis {
                requested_block,
                genesis_block,
            },
            ContinuityError::NoConsensusPointBefore { block_number } => ServiceError::Internal {
                message: format!(
                    "No consensus point (attestation or checkpoint) found before block {block_number}"
                ),
            },
            ContinuityError::AttestationIntervalNotConfigured { chain_key } => {
                ServiceError::Internal {
                    message: format!(
                        "Attestation interval not configured for chain_key {chain_key}"
                    ),
                }
            }
            ContinuityError::EmptyQuery => ServiceError::InvalidParameter {
                message: "Empty query: no block heights provided".to_string(),
            },
            ContinuityError::UpperBoundNotOnSourceChain {
                query_block: _,
                upper_block,
                current_block,
            } => ServiceError::BlockNotOnSourceChain {
                requested_block: upper_block,
                current_block,
            },
        }
    }
}

impl From<serde_json::Error> for ServiceError {
    fn from(e: serde_json::Error) -> Self {
        ServiceError::Internal {
            message: e.to_string(),
        }
    }
}

impl GetErrorType for ServiceError {
    fn error_type(&self) -> ErrorType {
        match self {
            ServiceError::AttestationsMissing { .. } => ErrorType::AttestationsMissing,
            ServiceError::QueryOutOfRange { .. } => ErrorType::QueryOutOfRange,
            ServiceError::TxIndexOutOfBounds { .. } => ErrorType::TxIndexOutOfBounds,
            ServiceError::RpcUnavailable { .. } => ErrorType::RpcUnavailable,
            ServiceError::MerkleError { .. } => ErrorType::MerkleError,
            ServiceError::InvalidParameter { .. } => ErrorType::InvalidParameter,
            ServiceError::Internal { .. } => ErrorType::Internal,
            ServiceError::TxHashLookupUnavailable { .. } => ErrorType::TxHashLookupUnavailable,
            ServiceError::TxHashNotFound { .. } => ErrorType::TxHashNotFound,
            ServiceError::BlockNotReady { .. } => ErrorType::BlockNotReady,
            ServiceError::BlockBeforeGenesis { .. } => ErrorType::BlockBeforeGenesis,
            ServiceError::BlockNotOnSourceChain { .. } => ErrorType::BlockNotOnSourceChain,
            ServiceError::EmptyProofQueries => ErrorType::EmptyProofQueries,
            ServiceError::TooManyProofQueries => ErrorType::TooManyProofQueries,
            ServiceError::TooManyTxQueriesInProofQuery => ErrorType::TooManyTxQueriesInProofQuery,
            ServiceError::EmptyTxHashes => ErrorType::EmptyTxHashes,
            ServiceError::TooManyTxHashes => ErrorType::TooManyTxHashes,
        }
    }
}
