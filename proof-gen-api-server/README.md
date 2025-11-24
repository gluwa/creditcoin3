# Proof Generation API Server

Rust-based API server component that provides on-demand continuity and merkle proof generation for Creditcoin Oracle queries. The server acts as a caching and proactive proof generation service, reducing latency for query verification operations.

## Launching the Database

```sh
docker compose -f ./proof-gen-api-server/docker-compose.yaml down
docker compose -f ./proof-gen-api-server/docker-compose.yaml up -d
```

## Building the Proof Gen Server

```sh
cargo b -r --features fast-runtime
```

## Launching Proof Gen Server: Local chain

```sh
cd proof-gen-api-server
../target/release/proof-gen-api-server \
    --cc3-key "//Alice"
```

## Launching Proof Gen Server: Devnet

```sh
cd proof-gen-api-server
../target/release/proof-gen-api-server \
    --cc3-key "//Alice" \
    --cc3-rpc-url wss://rpc.ccnext-devnet.creditcoin.network \
    --eth-rpc-url https://anvil.ccnext-devnet.creditcoin.network
```

## Continuity & Merkle Proof Endpoints (Internal Draft)

This crate exposes HTTP endpoints that produce continuity proofs and optional transaction merkle inclusion proofs.

Base path: `/api/v1`

Endpoints:

- `GET /proof/{chain_key}/{header_number}` – continuity proof for a header.
- `GET /proof/{chain_key}/{header_number}/{tx_index}` – continuity + merkle proof for the transaction at `tx_index` (supports empty block with index 0).
- `GET /proof-by-tx/{chain_key}/{tx_hash}` – placeholder for future reverse lookup.

Environment variables (subject to change):

- `USE_MOCK_PROVIDERS` – truthy values (`1`, `true`, `yes`) enable deterministic mock RPC providers.
- `RUST_LOG` – if set to `production` while mocks are enabled, startup is refused as a safety guard.
- `CC3_RPC_URL`, `ETH_RPC_URL` – real chain RPC endpoints when not using mocks.
- `POSTGRES_HOST`, `POSTGRES_PORT`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` – required for proof caching.
- `CC3_KEY` – mnemonic / key used for chain interactions where required.

Safety Guard:
Startup aborts if mocks are enabled alongside `RUST_LOG=production` to prevent accidental production deployment of synthetic data.

Example (mock mode) using env var for the key (matching docker-compose defaults):

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
export USE_MOCK_PROVIDERS=1
export CC3_KEY="dummy mnemonic"
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server
```

Example (mock mode) using CLI arg for the key:

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
export USE_MOCK_PROVIDERS=1
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server -- --cc3-key "dummy mnemonic"
```

Example (real providers) using env var for the key:

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
unset USE_MOCK_PROVIDERS
export CC3_KEY="your mnemonic"
export CC3_RPC_URL=ws://127.0.0.1:9944
export ETH_RPC_URL=http://127.0.0.1:8545
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server
```

Example (real providers) using CLI arg for the key:

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
unset USE_MOCK_PROVIDERS
export CC3_RPC_URL=ws://127.0.0.1:9944
export ETH_RPC_URL=http://127.0.0.1:8545
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server -- --cc3-key "your mnemonic"
```

Error Response Shape:

```jsonc
{
  "code": "TxIndexOutOfBounds", // string identifier
  "message": "tx_index 5 exceeds length 3", // human-readable detail
  "retriable": false // whether client should retry later
}
```

Note: This section is intentionally kept here (not in the repo root) until the API stabilizes.
