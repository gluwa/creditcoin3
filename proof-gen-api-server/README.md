# Proof Generation API Server

Rust-based API server component that provides on-demand continuity and merkle proof generation for Creditcoin Oracle queries. The server uses the indexer for fast proof generation and can fall back to building proofs from source chain data.

## Building the Proof Gen Server

```sh
cargo b -r --features fast-runtime
```

## Runtime Configuration & Startup

All configurable command line arguments are defined in `bin/server.rs` (see the `ProofGenApiServer` clap struct). For environment variables, copy `.env.example` to `.env` and adjust. The binary loads `.env` automatically.

Common pattern:

```bash
cp proof-gen-api-server/.env.example proof-gen-api-server/.env
cd proof-gen-api-server
cargo run -p proof-gen-api-server -- [FLAGS]
```

### Example: Local Development

Assumes you have a local Creditcoin3 node running on port 9944 and Anvil on port 8545. Pass the local RPC URLs:

```bash
cargo run -p proof-gen-api-server -- \
  --cc3-key "//Alice" \
  --cc3-rpc-url ws://localhost:9944 \
  --eth-rpc-url http://localhost:8545
```

If you have a `.env` file configured in `proof-gen-api-server/.env`, you can run from within that directory:

```bash
cd proof-gen-api-server
cargo run -p proof-gen-api-server -- \
  --cc3-key "//Alice" \
  --cc3-rpc-url ws://localhost:9944 \
  --eth-rpc-url http://localhost:8545
```

### Example: Devnet (real RPC endpoints)

```bash
cargo run -p proof-gen-api-server -- \
  --cc3-key "//Alice" \
  --cc3-rpc-url wss://rpc.ccnext-devnet.creditcoin.network \
  --eth-rpc-url https://anvil.ccnext-devnet.creditcoin.network
```

## Continuity & Merkle Proof Endpoints (Internal Draft)

This crate exposes HTTP endpoints that produce continuity proofs and optional transaction merkle inclusion proofs.

Base path: `/api/v1`

Endpoints:

- `GET /proof/{chain_key}/{header_number}/{tx_index}` – continuity + merkle proof for the transaction at `tx_index` (supports empty block with index 0).
- `GET /proof-by-tx/{chain_key}/{tx_hash}` – currently disabled (returns TxHashLookupUnavailable) until reverse lookup is implemented.

### API Documentation (Swagger)

When the server is running, interactive OpenAPI documentation is available at:

- **Swagger UI**: `http://localhost:3100/api/swagger` (default port; use `--bind-port` to change)
- **OpenAPI JSON**: `http://localhost:3100/api/swagger/openapi.json`

The Swagger UI lets you explore endpoints, view request/response schemas (including `continuityProof` and `merkleProof` structures), and try requests from the browser.

### Integration Testing

Integration tests exercise proof endpoints with Anvil. Run with:

```bash
cargo test -p proof-gen-api-server --features integration-tests
```

### Testing Using submit-proof.js

In addition to the unit tests within the proof-gen-api-server crate, you can test the server's functionality in non-mocked conditions with the following steps.

1. Follow the steps in `.github/CONTRIBUTING.md` up through step 4.
2. Follow `Building the Proof Gen Server` in this readme
3. Follow `Example: Local Development` in this readme to launch the proof gen server
4. Follow the steps from `### submit-proof.js` in `scripts/README.md`

## Indexer Integration

The server uses the indexer for fast proof generation by fetching pre-computed continuity proofs. If the indexer is unavailable or doesn't have the required data, the server falls back to building proofs from source chain data.

### Environment Configuration (.env)

You can optionally create a `.env` file in the `proof-gen-api-server/` directory to set environment variables. The server loads environment variables via `dotenvy`.

Guidelines:

- Keep stable defaults (ports, local RPC endpoints) in `.env`.
- Override frequently changed or sensitive values (mnemonics, indexer URLs) via CLI arguments or temporary exports when needed.
- Never commit your `.env` file to version control.

Production notes:

- Provide a non-development mnemonic for production deployments.
- Configure the indexer URL for optimal performance.

### Archiver URL (optional)

If you run the [archiver](../archiver/README.md) service (HTTP API that stores source-chain blocks and serves pre-computed Merkle roots), you can point the proof gen server at it so **continuity proofs** are built from those roots instead of fetching full blocks over Ethereum RPC.

- **CLI:** `--archiver-url http://<host>:<port>` (example: `http://localhost:8080`)
- **Environment:** `ARCHIVER_URL` (same semantics; use one or the other)

Ethereum RPC (`--eth-rpc-url` / `ETH_RPC_URL`) remains required: the server still uses it for chain tip and other paths that do not go through the archiver.

Example (local archiver on port 8080):

```bash
cargo run -p proof-gen-api-server -- \
  --cc3-key "//Alice" \
  --cc3-rpc-url ws://localhost:9944 \
  --eth-rpc-url http://localhost:8545 \
  --archiver-url http://localhost:8080
```

Error Response Shape:

```jsonc
{
  "code": "TxIndexOutOfBounds", // string identifier
  "message": "tx_index 5 exceeds length 3", // human-readable detail
  "retriable": false // whether client should retry later
}
```

**Eager Proof Generation**: When requesting a block that hasn't been attested yet, the server
will predict the next attestation using the attestation interval and generate an "eager" proof.
This proof will become verifiable once the predicted attestation is created on-chain.

In rare cases where no attestations exist at all, a `BlockNotReady` error is returned:

```jsonc
{
  "code": "BlockNotReady",
  "message": "The continuity proof cannot be created because block 35 is not attested to yet. Last attested block: 30",
  "retriable": true,
  "block_number": 35, // the requested block number
  "last_attested_block": 30 // the highest block that has been attested
}
```

Note: This section is intentionally kept here (not in the repo root) until the API stabilizes.
