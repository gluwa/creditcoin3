# Continuity

Continuity proof generation library for the Creditcoin3 ecosystem.

## Overview

This crate provides the core logic for building continuity proofs that cryptographically link source chain blocks to Creditcoin3 attestations. It's used by both the `proof-gen-api-server` and `query-cli` to generate proofs for oracle queries.

## Architecture

### Data Sources

The builder supports two data sources with automatic fallback:

1. **Indexer (Fast)** - Fetches pre-computed continuity proofs from the CC3 indexer GraphQL API
2. **CC3 Chain (Fallback)** - Builds proofs by querying CC3 chain directly and fetching source chain blocks

### Module Structure

```
src/
├── lib.rs              - Public API & re-exports
├── config.rs           - Configuration types
├── errors.rs           - Custom error types
├── attestation.rs      - Attestation info types
├── proof.rs            - Proof result types
├── indexer.rs          - IndexerProvider trait
├── rpc.rs              - CcRpcProvider & EthRpcProvider traits
├── mocks.rs            - Mock providers for testing
└── builder/
    ├── mod.rs          - ContinuityBuilder struct & constructors
    ├── build.rs        - Core proof building (indexer & CC3 paths)
    ├── cc3.rs          - CC3 chain data fetching
    └── bounds.rs       - Attestation bounds finding (CC3 path)
```

## Usage

### Basic Usage

```rust
use continuity::{ContinuityBuilder, ContinuityConfig};

// Create configuration (automatically fetches both intervals from chain)
let config = ContinuityConfig::builder()
    .cc3_rpc_url("wss://rpc.creditcoin.network")
    .eth_rpc_url("https://eth-rpc.example.com")
    .chain_key(chain_key)
    .fetch_intervals()
    .await?;

// Build the continuity builder
let builder = ContinuityBuilder::new(config).await?;

// Get attestation endpoints for a query
let query_height = 100;
let (lower_attestation, upper_attestation, _) = 
    builder.get_endpoints(&[query_height], None).await?;

// Build the continuity proof
let proof = builder
    .build_for_single_query(query_height, lower_attestation, upper_attestation)
    .await?;
```

### With Indexer (Recommended)

```rust
use continuity::{ContinuityBuilder, ContinuityConfig};
use indexer_client::IndexerClient;
use std::sync::Arc;

let config = ContinuityConfig::new(
    cc3_rpc_url,
    cc3_key,
    eth_rpc_url,
    chain_key,
    checkpoint_interval,
);

// Create indexer client
let indexer = Arc::new(IndexerClient::new(indexer_url)?);

// Create builder with indexer
let builder = ContinuityBuilder::new_with_indexer(
    config,
    cc_client,
    eth_client,
    Some(indexer),
);
```

### With Block Caching (for high-performance)

```rust
use continuity::ContinuityBuilder;
use eth::block_cache::BlockCacheConfig;

let cache_config = BlockCacheConfig {
    redis_url: "redis://localhost:6379".to_string(),
    metrics_registry: None,
};

let builder = ContinuityBuilder::new_with_block_caching(config, cache_config).await?;
```

### Batch Queries

For multiple queries, build a single continuity proof that covers all blocks:

```rust
let query_heights = vec![100, 105, 110];
let (lower, upper, _) = builder.get_endpoints(&query_heights, None).await?;
let proof = builder
    .build_for_batch_queries(&query_heights, lower, upper)
    .await?;
```

## Key Concepts

### Attestations

Attestations are consensus points on the Creditcoin3 chain that anchor source chain block digests. They occur at regular intervals (e.g., every 10 blocks).

### Checkpoints

Checkpoints are special attestations that occur at longer intervals (e.g., every 10 attestations = 100 blocks). They provide long-term storage of attestation data.

### Continuity Proof

A continuity proof is a chain of blocks that:
1. Starts at `queryHeight - 1` (to prove the query block's parent)
2. Includes the query block(s)
3. Ends at the next attestation after the query

This allows on-chain verification that a source chain block was correctly attested.

### Endpoints

The "endpoints" are the lower and upper attestation boundaries for a query:
- **Lower endpoint**: The attestation at or before `min_query - 1`
- **Upper endpoint**: The attestation after `max_query`

## Provider Traits

### `CcRpcProvider`

Abstraction over Creditcoin3 RPC operations:
- Fetch attestations and checkpoints
- Query attestation intervals
- Get chain metadata

### `EthRpcProvider`

Abstraction over source chain (Ethereum/EVM) RPC operations:
- Build continuity blocks from source chain
- Fetch transaction data
- Get block information

### `IndexerProvider`

Abstraction over indexer GraphQL API:
- Fetch pre-computed continuity blocks
- Query attestation metadata
- Batch fetch attestations in range

## Error Handling

The crate uses custom error types (`ContinuityError`) with specific variants:

- `NoAttestations` - No attestations found for chain
- `BlockNotReady` - Block not yet attested (retriable)
- `BlockBeforeGenesis` - Block before attestation system initialized
- `EmptyQuery` - No query heights provided
- `UpperBoundNotOnSourceChain` - Predicted attestation doesn't exist yet

All builder methods return `ContinuityResult<T>` which is an alias for `Result<T, ContinuityError>`.

## Testing

The crate includes mock providers for testing:

```rust
use continuity::mocks::make_mock_providers;

let chain_key = 2;
let (cc_provider, eth_provider) = make_mock_providers(chain_key);
let builder = ContinuityBuilder::new_with_providers(config, cc_provider, eth_provider);
```

Run tests:

```bash
cargo test -p continuity
```

## Features

- `block_cache` - Enable Redis-based block caching for ETH client (requires Redis)

## Dependencies

This crate depends on:
- `attestor-primitives` - Block and attestation types
- `cc-client` - Creditcoin3 RPC client
- `eth` - Ethereum RPC client
- `indexer-client` - Optional, used by consumers for indexer integration

## Performance Considerations

### With Indexer (Fast Path)
- Single GraphQL query fetches pre-computed continuity blocks
- Typical response time: ~100-500ms
- No source chain RPC calls needed

### Without Indexer (Slow Path)
- Fetches all attestations from CC3 chain
- Builds blocks from source chain RPC
- Typical response time: ~2-10 seconds depending on chain state

### Block Caching
Enable `block_cache` feature and configure Redis to cache source chain blocks, reducing proof generation time by ~70% for repeated queries.

## Examples

See `tests/builder_proof.rs` for working examples.
