use anyhow::Error as AnyError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("attestations missing for chain {chain_key}")]
    AttestationsMissing { chain_key: u64 },
    #[error("query height {height} out of range")]
    QueryOutOfRange { height: u64 },
    #[error("tx_index {tx_index} out of bounds (len={len})")]
    TxIndexOutOfBounds { tx_index: u64, len: usize },
    #[error("rpc unavailable: {message}")]
    RpcUnavailable { message: String },
    #[error("database error: {message}")]
    DbError { message: String },
    #[error("merkle proof generation failed: {message}")]
    MerkleError { message: String },
    #[error("invalid parameter: {message}")]
    InvalidParameter { message: String },
    #[error("internal error: {message}")]
    Internal { message: String },
}

impl ServiceError {
    pub fn retriable(&self) -> bool {
        matches!(
            self,
            ServiceError::RpcUnavailable { .. } | ServiceError::DbError { .. }
        )
    }
    pub fn code(&self) -> &'static str {
        match self {
            ServiceError::AttestationsMissing { .. } => "AttestationsMissing",
            ServiceError::QueryOutOfRange { .. } => "QueryOutOfRange",
            ServiceError::TxIndexOutOfBounds { .. } => "TxIndexOutOfBounds",
            ServiceError::RpcUnavailable { .. } => "RpcUnavailable",
            ServiceError::DbError { .. } => "DatabaseError",
            ServiceError::MerkleError { .. } => "MerkleError",
            ServiceError::InvalidParameter { .. } => "InvalidParameter",
            ServiceError::Internal { .. } => "Internal",
        }
    }
}

impl From<AnyError> for ServiceError {
    fn from(e: AnyError) -> Self {
        ServiceError::Internal {
            message: e.to_string(),
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
