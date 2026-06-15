# Indexer Client

GraphQL client for querying the Creditcoin3 attestations indexer.

## Overview

This crate provides a well-structured client for fetching attestation continuity proofs and related data from the CC3 indexer GraphQL API. It implements the `IndexerProvider` trait from the `continuity` crate, allowing it to be used as a data source for building continuity proofs.

## Features

- **Custom Error Types**: Proper error handling with `IndexerError` using `thiserror`
- **GraphQL Queries**: Pre-defined queries for common operations
- **Block Digest Recomputation**: Ensures digest correctness by recomputing instead of trusting indexer values
- **Full Test Coverage**: Comprehensive test suite with mock responses

## Usage

```rust
use indexer_client::{IndexerClient, IndexerError};

// Create a client
let client = IndexerClient::new("https://indexer.example.com/graphql".to_string())?;

// Fetch a continuity proof
let proof = client.get_continuity_proof(chain_key, header_number).await?;

// Fetch continuity blocks
let blocks = client.get_continuity_blocks(chain_key, header_number).await?;
```

## Integration with Continuity Builder

The `IndexerClient` implements `IndexerProvider`, making it easy to integrate with `ContinuityBuilder`:

```rust
use indexer_client::IndexerClient;
use continuity::ContinuityBuilder;
use std::sync::Arc;

let indexer = Arc::new(IndexerClient::new(indexer_url)?);
let builder = ContinuityBuilder::new_with_indexer(config, cc3_client, eth_client, Some(indexer));
```

## Architecture

```
src/
├── client.rs   - IndexerClient implementation
├── error.rs    - Custom error types
├── queries.rs  - GraphQL query definitions
├── types.rs    - Request/response types
├── tests.rs    - Test suite
└── lib.rs      - Public API
```

## Error Handling

All methods return `Result<T, IndexerError>` with specific error variants:

- `HttpRequest` - Network failures
- `GraphQLRequestFailed` - HTTP errors with status codes
- `GraphQLErrors` - GraphQL-level errors
- `ParseResponse` - JSON deserialization failures
- `InvalidHex` - Hex string parsing errors
- `MissingField` - Required fields missing from responses
- `InvalidEndpoint` - Configuration errors
- `MissingPrevDigest` - Continuity proof validation errors

## Testing

The crate includes comprehensive tests using `wiremock` for mocking the GraphQL API:

```bash
cargo test -p indexer-client
```
