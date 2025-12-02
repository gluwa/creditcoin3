use continuity::ContinuityError;
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
    #[error("tx hash reverse lookup unavailable for {tx_hash}")]
    TxHashLookupUnavailable { tx_hash: String },
    #[error("tx hash not found: {tx_hash}")]
    TxHashNotFound { tx_hash: String },
    #[error("The continuity proof cannot be created because block {block_number} is not attested to yet")]
    BlockNotReady {
        block_number: u64,
        current_block: u64,
    },
}

impl ServiceError {
    pub fn retriable(&self) -> bool {
        matches!(
            self,
            ServiceError::RpcUnavailable { .. }
                | ServiceError::DbError { .. }
                | ServiceError::BlockNotReady { .. }
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
            ServiceError::TxHashLookupUnavailable { .. } => "TxHashLookupUnavailable",
            ServiceError::TxHashNotFound { .. } => "TxHashNotFound",
            ServiceError::BlockNotReady { .. } => "BlockNotReady",
        }
    }
}

impl From<ContinuityError> for ServiceError {
    fn from(e: ContinuityError) -> Self {
        match e {
            ContinuityError::BlockNotReady {
                block_number,
                current_block,
            } => ServiceError::BlockNotReady {
                block_number,
                current_block,
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
