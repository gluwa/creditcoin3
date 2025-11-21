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