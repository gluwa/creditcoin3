# Proof Generation API Server
Rust-based API server component that provides on-demand continuity and merkle proof generation for Creditcoin Oracle queries. The server acts as a caching and proactive proof generation service, reducing latency for query verification operations.

## Launching the Database
```sh
docker compose -f ./proof-gen-api-server/docker-compose.yaml down
docker compose -f ./proof-gen-api-server/docker-compose.yaml up -d
```

## Launching the Proof Gen Server
```sh
cargo b -r --features fast-runtime
cd proof-gen-api-server
../target/release/proof-gen-api-server
```