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
- `GET /proof-by-tx/{chain_key}/{tx_hash}` – currently disabled (returns TxHashLookupUnavailable) until reverse lookup is implemented.

Environment variables (subject to change):

- `RUST_LOG` – if set to `production` while the CLI flag `--use-mock-providers` is passed, startup is refused as a safety guard.
- `CC3_RPC_URL`, `ETH_RPC_URL` – real chain RPC endpoints when not using mocks.
- `POSTGRES_HOST`, `POSTGRES_PORT`, `POSTGRES_USER`, `POSTGRES_PASSWORD`, `POSTGRES_DB` – required for proof caching.
- `CC3_KEY` – mnemonic / key used for chain interactions where required.

### Environment Configuration (.env)

Instead of exporting all variables manually, you can create a `.env` file (not committed) based on the provided `.env.example` in this directory:

```bash
cp proof-gen-api-server/.env.example proof-gen-api-server/.env
```

Adjust the values (especially `CC3_KEY`, any real RPC URLs, and Postgres credentials). The server loads environment variables via `dotenvy`, so running `cargo run -p proof-gen-api-server` inside `proof-gen-api-server/` will automatically pick them up.

Guidelines:

- Keep stable defaults (ports, local RPC endpoints) in `.env`.
- Override frequently changed or sensitive values (mnemonics, feature flags) via CLI arguments or temporary exports when needed.
- Never commit your personal `.env`; only `.env.example` lives in version control.

Production notes:

- Remove or set `USE_MOCK_PROVIDERS=0` (or unset) before deploying.
- Set `RUST_LOG=production` or `prod` (enables production guard against mocks).
- Provide secure Postgres credentials and a non-development mnemonic.

Safety Guard:
Startup aborts if mocks are enabled alongside `RUST_LOG=production` to prevent accidental production deployment of synthetic data.

Example (mock mode via CLI flag) using env var for the key (matching docker-compose defaults):

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
export CC3_KEY="dummy mnemonic"
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server -- --use-mock-providers
```

Example (mock mode) overriding key via CLI arg:

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server -- --cc3-key "dummy mnemonic" --use-mock-providers
```

Example (real providers) using env var for the key (omit mock flag):

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
export CC3_KEY="your mnemonic"
export CC3_RPC_URL=ws://127.0.0.1:9944
export ETH_RPC_URL=http://127.0.0.1:8545
export POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db
cargo run -p proof-gen-api-server
```

Example (real providers) using CLI arg for the key (omit mock flag):

```bash
docker compose -f proof-gen-api-server/docker-compose.yaml up -d
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
