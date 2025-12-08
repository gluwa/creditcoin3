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

## Runtime Configuration & Startup

All configurable command line arguments are defined in `bin/proof-gen-server.rs` (see the `ProofGenApiServer` clap struct). For environment variables, copy `.env.example` to `.env` and adjust. The binary loads `.env` automatically.

Common pattern:

```bash
cp proof-gen-api-server/.env.example proof-gen-api-server/.env
cd proof-gen-api-server
cargo run -p proof-gen-api-server -- [FLAGS]
```

### Example: Local Development

Assumes you have a local Creditcoin3 node running on port 9944 and Anvil on port 8545. Set the database environment variables and pass the local RPC URLs:

```bash
POSTGRES_HOST=localhost POSTGRES_PORT=5433 POSTGRES_USER=postgres POSTGRES_PASSWORD=password POSTGRES_DB=proofs_db \
cargo run -p proof-gen-api-server -- \
  --cc3-key "//Alice" \
  --cc3-rpc-url ws://localhost:9944 \
  --eth-rpc-url http://localhost:8545
```

Alternatively, if you have a `.env` file configured in `proof-gen-api-server/.env` with the database settings, you can run from within that directory:

```bash
cd proof-gen-api-server
cargo run -p proof-gen-api-server -- \
  --cc3-key "//Alice" \
  --cc3-rpc-url ws://localhost:9944 \
  --eth-rpc-url http://localhost:8545
```

### Example: Devnet (real RPC endpoints)

Override RPC URLs while using the same `.env` for database configuration:

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

- `GET /proof/{chain_key}/{header_number}` – continuity proof for a header.
- `GET /proof/{chain_key}/{header_number}/{tx_index}` – continuity + merkle proof for the transaction at `tx_index` (supports empty block with index 0).
- `GET /proof-by-tx/{chain_key}/{tx_hash}` – currently disabled (returns TxHashLookupUnavailable) until reverse lookup is implemented.

### E2E Testing

An E2E test exercises all three proof endpoints with Anvil and ephemeral Postgres. Prerequisites: [Foundry](https://book.getfoundry.sh/getting-started/installation) (`anvil`, `cast`) and Docker. Run with:

```bash
cargo test -p proof-gen-api-server --test anvil_e2e --features e2e-tests
```

### Testing Using submit-proof.js

In addition to the unit tests within the proof-gen-api-server crate, you can test the server's functionality in non-mocked conditions with the following steps.

1. Follow the steps in `.github/CONTRIBUTING.md` up through step 4.
2. Follow `Launching the Database` in this readme
2. Follow `Building the Proof Gen Server` in this readme
3. Follow `Example: Local Development` in this readme to launch the proof gen server
4. Follow the steps from `### submit-proof.js` in `scripts/README.md`

## Caching

The server caches generated continuity proofs in Postgres to avoid recomputation on subsequent requests.

- Storage: the `continuity_proofs` table persists continuity proofs (JSONB).
- Writes: inserts are performed asynchronously (fire-and-forget) with upsert semantics, so concurrent requests race safely.
- Reads: endpoints first attempt to deserialize cached JSON; if successful, responses include `"cached": true`. If cache is missing or deserialization fails, proofs are rebuilt and `"cached": false` is returned.

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

- Provide secure Postgres credentials and a non-development mnemonic.

Error Response Shape:

```jsonc
{
  "code": "TxIndexOutOfBounds", // string identifier
  "message": "tx_index 5 exceeds length 3", // human-readable detail
  "retriable": false // whether client should retry later
}
```

Note: This section is intentionally kept here (not in the repo root) until the API stabilizes.
