use axum::http::StatusCode;
use axum::Json;
use continuity::ContinuityError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::prom::{ErrorType, GetErrorType};

/// Render the user-facing message for [`ServiceError::BlockNotOnSourceChain`].
///
/// Two cases share this error variant:
/// 1. `requested_block > current_block` — the block has not been mined yet.
/// 2. `requested_block <= current_block` and within `confirmation_depth` of the
///    tip — the block exists but is inside the reorg-protection window.
fn format_block_not_on_source_chain(
    requested_block: u64,
    current_block: u64,
    confirmation_depth: u64,
) -> String {
    if requested_block > current_block {
        format!(
            "Block {requested_block} does not exist on the source chain yet. \
             Current source chain height: {current_block}"
        )
    } else {
        let confirmed_block = current_block.saturating_sub(confirmation_depth);
        format!(
            "Block {requested_block} is within the source chain's reorg-protection window \
             ({confirmation_depth} block(s)) and is not yet confirmed. \
             Current source chain height: {current_block}; latest confirmed block: {confirmed_block}. \
             Retry once the chain advances past block {requested_block}."
        )
    }
}

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
    #[error("block {block_number}: source chain RPC returned data inconsistent with the block header (mixed endpoints, unsupported trie layout, or bad archive payload); cannot build proof")]
    UnsupportedBlockFormat { block_number: u64 },
    #[error("unknown or unsupported chain_key {chain_key} for this server")]
    UnknownChain { chain_key: u64 },
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
    /// The block at `height` contains no transactions, so there is no transaction proof to
    /// build. The block-prover precompile rejects empty transaction bytes, so returning the
    /// previous "empty merkle proof at tx_index=0" payload only produced proofs that fail
    /// on-chain verification. Callers wanting a continuity-only attestation for an empty
    /// block should use a continuity-only endpoint instead.
    #[error("block {height} is empty; tx proof unavailable (use a continuity-only proof)")]
    EmptyBlockTxProof { height: u64 },
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
    #[error("Block {requested_block} is before or at attestation genesis block {genesis_block}. Cannot generate proofs for blocks before the attestation system was initialized.")]
    BlockBeforeOrAtGenesis {
        requested_block: u64,
        genesis_block: u64,
    },
    /// Returned when a requested block cannot be served because it is either
    /// past the source chain tip *or* within the per-chain `block_confirmation_depth`
    /// reorg-protection window. `confirmation_depth = 0` means there is no reorg
    /// window, so this strictly indicates the block is past the tip.
    #[error("{}", format_block_not_on_source_chain(*requested_block, *current_block, *confirmation_depth))]
    BlockNotOnSourceChain {
        requested_block: u64,
        current_block: u64,
        confirmation_depth: u64,
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
    #[error("Batch span too large: requested span {span} blocks (from block {from_block} to {to_block}) exceeds the maximum allowed span of {max_span} blocks")]
    BatchSpanTooLarge {
        from_block: u64,
        to_block: u64,
        span: u64,
        max_span: u64,
    },
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
            ServiceError::UnsupportedBlockFormat { .. } => "UnsupportedBlockFormat",
            ServiceError::UnknownChain { .. } => "UnknownChain",
            ServiceError::AttestationsMissing { .. } => "AttestationsMissing",
            ServiceError::QueryOutOfRange { .. } => "QueryOutOfRange",
            ServiceError::TxIndexOutOfBounds { .. } => "TxIndexOutOfBounds",
            ServiceError::EmptyBlockTxProof { .. } => "EmptyBlockTxProof",
            ServiceError::RpcUnavailable { .. } => "RpcUnavailable",
            ServiceError::MerkleError { .. } => "MerkleError",
            ServiceError::InvalidParameter { .. } => "InvalidParameter",
            ServiceError::Internal { .. } => "Internal",
            ServiceError::TxHashLookupUnavailable { .. } => "TxHashLookupUnavailable",
            ServiceError::TxHashNotFound { .. } => "TxHashNotFound",
            ServiceError::BlockNotReady { .. } => "BlockNotReady",
            ServiceError::BlockBeforeOrAtGenesis { .. } => "BlockBeforeOrAtGenesis",
            ServiceError::BlockNotOnSourceChain { .. } => "BlockNotOnSourceChain",
            ServiceError::EmptyProofQueries => "EmptyProofQueries",
            ServiceError::TooManyProofQueries => "TooManyProofQueries",
            ServiceError::TooManyTxQueriesInProofQuery => "TooManyTxQueriesInProofQuery",
            ServiceError::TooManyTxHashes => "TooManyTxHashes",
            ServiceError::EmptyTxHashes => "EmptyTxHashes",
            ServiceError::BatchSpanTooLarge { .. } => "BatchSpanTooLarge",
        }
    }

    pub fn status_code(&self) -> StatusCode {
        match self {
            Self::UnknownChain { .. } => StatusCode::BAD_REQUEST,
            Self::AttestationsMissing { .. } => StatusCode::NOT_FOUND,
            Self::QueryOutOfRange { .. }
            | Self::TxIndexOutOfBounds { .. }
            | Self::InvalidParameter { .. }
            | Self::BlockBeforeOrAtGenesis { .. }
            | Self::EmptyProofQueries
            | Self::TooManyProofQueries
            | Self::TooManyTxQueriesInProofQuery
            | Self::EmptyTxHashes
            | Self::TooManyTxHashes
            | Self::BatchSpanTooLarge { .. } => StatusCode::BAD_REQUEST,
            Self::RpcUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
            Self::TxHashLookupUnavailable { .. } => StatusCode::NOT_IMPLEMENTED,
            Self::TxHashNotFound { .. } | Self::BlockNotOnSourceChain { .. } => {
                StatusCode::NOT_FOUND
            }
            // Block exists but is not yet attested. The request is well-formed but
            // cannot be processed in the current state, so 422 Unprocessable Entity
            // is more accurate than 404 Not Found and lets clients distinguish
            // "keep waiting / retry" from "this will never exist".
            // Same semantics as BlockNotReady: request may be valid but payload cannot be processed.
            // EmptyBlockTxProof joins this group: the request is well-formed and the block
            // exists, but no transaction proof can ever be produced for an empty block.
            Self::UnsupportedBlockFormat { .. }
            | Self::BlockNotReady { .. }
            | Self::EmptyBlockTxProof { .. } => StatusCode::UNPROCESSABLE_ENTITY,
            Self::MerkleError { .. } | Self::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn into_response(self) -> (StatusCode, Json<ErrorResponse>) {
        let status = self.status_code();
        let response = ErrorResponse::from_service_error(&self);

        // Structured log for every failed API response; inherits `http_request` span fields (e.g. request_id).
        if status.is_server_error() {
            tracing::error!(
                http_status = %status,
                error_code = %response.code,
                error_message = %response.message,
                retriable = response.retriable,
                block_number = ?response.block_number,
                last_attested_block = ?response.last_attested_block,
                "🌐 ❌ proof API returning server error"
            );
        } else {
            tracing::warn!(
                http_status = %status,
                error_code = %response.code,
                error_message = %response.message,
                retriable = response.retriable,
                block_number = ?response.block_number,
                last_attested_block = ?response.last_attested_block,
                "🌐 ⚠️  proof API returning client error"
            );
        }

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
            ContinuityError::BlockBeforeOrAtGenesis {
                requested_block,
                genesis_block,
            } => ServiceError::BlockBeforeOrAtGenesis {
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
                // Upper-bound checks always trigger because `upper_block > current_block`,
                // i.e. the predicted attestation upper bound has not been mined yet.
                // Reorg-window confirmation depth is irrelevant here.
                confirmation_depth: 0,
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
            ServiceError::UnsupportedBlockFormat { .. } => ErrorType::UnsupportedBlockFormat,
            ServiceError::UnknownChain { .. } => ErrorType::UnknownChain,
            ServiceError::AttestationsMissing { .. } => ErrorType::AttestationsMissing,
            ServiceError::QueryOutOfRange { .. } => ErrorType::QueryOutOfRange,
            ServiceError::TxIndexOutOfBounds { .. } => ErrorType::TxIndexOutOfBounds,
            ServiceError::EmptyBlockTxProof { .. } => ErrorType::EmptyBlockTxProof,
            ServiceError::RpcUnavailable { .. } => ErrorType::RpcUnavailable,
            ServiceError::MerkleError { .. } => ErrorType::MerkleError,
            ServiceError::InvalidParameter { .. } => ErrorType::InvalidParameter,
            ServiceError::Internal { .. } => ErrorType::Internal,
            ServiceError::TxHashLookupUnavailable { .. } => ErrorType::TxHashLookupUnavailable,
            ServiceError::TxHashNotFound { .. } => ErrorType::TxHashNotFound,
            ServiceError::BlockNotReady { .. } => ErrorType::BlockNotReady,
            ServiceError::BlockBeforeOrAtGenesis { .. } => ErrorType::BlockBeforeOrAtGenesis,
            ServiceError::BlockNotOnSourceChain { .. } => ErrorType::BlockNotOnSourceChain,
            ServiceError::EmptyProofQueries => ErrorType::EmptyProofQueries,
            ServiceError::TooManyProofQueries => ErrorType::TooManyProofQueries,
            ServiceError::TooManyTxQueriesInProofQuery => ErrorType::TooManyTxQueriesInProofQuery,
            ServiceError::EmptyTxHashes => ErrorType::EmptyTxHashes,
            ServiceError::TooManyTxHashes => ErrorType::TooManyTxHashes,
            ServiceError::BatchSpanTooLarge { .. } => ErrorType::BatchSpanTooLarge,
        }
    }
}
