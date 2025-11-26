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

### Example: Local Development (mock providers)

Assumes the default `.env` values (local Postgres, `//Alice` mnemonic). Enables deterministic mock providers:

```bash
cargo run -p proof-gen-api-server -- --cc3-key "//Alice" --use-mock-providers
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
cargo test -p proof-gen-api-server --test anvil_e2e --features e2e-tests -- --ignored
```

## Caching

The server caches generated proofs in Postgres to avoid recomputation on subsequent requests.

- Storage: the `proofs` table persists continuity and transaction merkle proofs (JSONB), plus `merkle_root` and `tx_hash` (hex strings).
- Keys:
  - Block-level continuity (no tx index): `(chain_key, header_number)` where `tx_index IS NULL`.
  - Tx-level continuity + merkle: `(chain_key, header_number, tx_index)` where `tx_index IS NOT NULL`.
  - Reverse lookup by tx-hash: matches on `tx_hash`.
- Writes: inserts are performed asynchronously (fire-and-forget) with upsert semantics, so concurrent requests race safely.
- Reads: endpoints first attempt to deserialize cached JSON; if successful, responses include `"cached": true`. If cache is missing or deserialization fails, proofs are rebuilt and `"cached": false` is returned.

Development/testing fallback:

- When `INMEM_DB_FALLBACK=true|1` is set, and Postgres is unavailable, proofs are stored in an in-memory cache for the process lifetime. This is intended only for local/tests and should not be enabled in production or CI.
- In containers, rely on Postgres; `.env.docker` does not set the fallback.

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
