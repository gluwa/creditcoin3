//! Error types for the indexer client

/// Errors that can occur when interacting with the indexer
#[derive(Debug, thiserror::Error)]
pub enum IndexerError {
    #[error("HTTP request failed: {0}")]
    HttpRequest(#[from] reqwest::Error),

    #[error("GraphQL request failed with status {status}: {body}")]
    GraphQLRequestFailed { status: u16, body: String },

    #[error("GraphQL errors: {0}")]
    GraphQLErrors(String),

    #[error("Failed to parse response: {0}")]
    ParseResponse(#[from] serde_json::Error),

    #[error("Invalid hex string for {field}: {error}")]
    InvalidHex { field: String, error: String },

    #[error("Failed to parse {field} as integer: {error}")]
    ParseInt { field: String, error: String },

    #[error("Missing required field: {0}")]
    MissingField(String),

    #[error("Invalid endpoint: {0}")]
    InvalidEndpoint(String),

    #[error("First block in continuity proof must have prev_digest")]
    MissingPrevDigest,

    #[error("Empty continuity proof")]
    EmptyProof,

    #[error("Failed to build HTTP client: {0}")]
    ClientBuild(String),

    #[error("Invalid indexer data: {message}")]
    InvalidIndexerData { message: String },
}
