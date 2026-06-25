# Archiver

Continuously archives source chain blocks, computes merkle roots, and serves the data over HTTP. Designed for gap-free archival with automatic reconnection and retry on RPC failures.

## Features

- **Gap-free archival** — blocks are stored in order with gap detection and backfill support
- **Automatic reconnection** — handles both clean disconnects and stale WebSocket connections (120s timeout)
- **Concurrent fetching** — configurable parallelism for block fetching (IO) and merkle root computation (CPU)
- **Resume on restart** — persists progress in a sled database; picks up where it left off
- **HTTP API** — serves archived roots and proof inputs for the continuity proof pipeline
- **Digest caching** — caches chained digests at regular intervals to avoid replaying from genesis

## Usage

```bash
RUST_LOG=debug target/release/archiver \
  --rpc-http http://localhost:8545 \
  --rpc-ws ws://localhost:8545 \
  --chain-key 2 \
  --start-height 0 \
  --api-bind 0.0.0.0:8080
```

All flags can also be set via environment variables (see below).

## Configuration

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--rpc-http` | `RPC_HTTP` | *(required)* | HTTP RPC endpoint for block fetching |
| `--rpc-ws` | `RPC_WS` | *(required)* | WebSocket RPC endpoint for new-head subscriptions |
| `--cc3-rpc_url` | `CC3_RPC_URL` | `ws://localhost:9944` | Url for connecting to CC3 chain |
| `--chain-key` | `CHAIN_KEY` | *(none)* | Chain key for supported chains entry of the chain we're archiving |
| `--start-height` | `START_HEIGHT` | `0` | Block height to start from (ignored if DB has progress) |
| `--end-height` | `END_HEIGHT` | *(none)* | Stop after this block (inclusive). Omit to follow the tip |
| `--max-fetch-tasks` | `MAX_FETCH_TASKS` | `8` | Max concurrent block fetch tasks (IO-bound) |
| `--max-api-range` | `MAX_API_RANGE` | `1000` | Max block range per `/roots` API request |
| `--max-api-concurrency` | `MAX_API_CONCURRENCY` | `16` | Max in-flight `/roots` requests; excess rejected with HTTP 429 |
| `--stream-timeout-secs` | `STREAM_TIMEOUT_SECS` | `120` | Seconds before treating a stream as stalled |
| `--sled-db-path` | `SLED_DB_PATH` | `./data/roots.sled` | Path to the sled database directory |
| `--api-bind` | `API_BIND` | `0.0.0.0:8080` | HTTP API bind address |
| `--flush-every` | `FLUSH_EVERY` | `10000` | Flush database to disk every N blocks |
| `--backfill` | — | `false` | Scan for gaps and fill them before resuming |
| `--finalization_lag_override` | - | *(none)* | Configurable finalization lag override |

A `.env` file in the working directory is loaded automatically.

## API Endpoints

### `GET /status`

Returns archiver status.

```json
{
  "latest_archived_block": 1234567,
  "total_blocks": 1234568
}
```

### `GET /roots/latest`

Returns the latest archived block number.

```json
{
  "latest_block": 1234567
}
```

### `GET /roots?from=100&to=200`

Returns merkle roots for an inclusive block range (max `MAX_API_RANGE` blocks per request, default 1,000).

The endpoint is also concurrency-limited: at most `MAX_API_CONCURRENCY` (default 16) requests are served
at once. Requests beyond that limit are rejected immediately with `429 Too Many Requests` rather than
queued, protecting the archiver from range-scan fan-out overload.

```json
[
  { "block_number": 100, "merkle_root": "0x..." },
  { "block_number": 101, "merkle_root": "0x..." }
]
```

## Architecture

```
Chain (WS) ──► StreamRoots ──► Merkle root computation ──► Sled DB ──► HTTP API
                  │                                            ▲
                  └── auto-reconnect + timeout ────────────────┘
                       (exponential backoff)              (resume height)
```

1. **StreamRoots** subscribes to new block headers via WebSocket and fetches full blocks via HTTP
2. Blocks are merkleized in parallel using `spawn_blocking` to avoid blocking the async runtime
3. Roots are batched and written to sled in height order
4. On restart, the archiver reads the latest stored height and resumes from there
5. The `--backfill` flag scans for any gaps and fills them before continuing

## Reconnection

The archiver handles two failure modes:

- **Clean disconnect** — the WS stream returns `None`, triggering immediate reconnection with exponential backoff
- **Stale connection** — the WS stream hangs (server stops sending headers without closing the socket). A 120-second timeout detects this and forces reconnection
