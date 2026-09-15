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
| `--rpc-http` | `RPC_HTTP` | *(required)* | HTTP RPC endpoint for chain-head tracking and the canonical-anchor check |
| `--rpc-ws` | `RPC_WS` | *(required)* | WebSocket RPC endpoint for the new-head subscription and block fetching |
| `--rpc-fallback-urls` | `RPC_FALLBACK_URLS` | *(none)* | Comma-separated extra RPCs tried in order when the primary returns "not found" or a transport error for a block fetch; must serve the same chain id; if any is unreachable at dial time the archiver warns and continues with the primary alone |
| `--cc3-rpc_url` | `CC3_RPC_URL` | `ws://localhost:9944` | Url for connecting to CC3 chain |
| `--chain-key` | `CHAIN_KEY` | *(none)* | Chain key for supported chains entry of the chain we're archiving |
| `--start-height` | `START_HEIGHT` | `0` | Block height to start from (ignored if DB has progress) |
| `--end-height` | `END_HEIGHT` | *(none)* | Stop after this block (inclusive). Omit to follow the tip |
| `--max-fetch-tasks` | `MAX_FETCH_TASKS` | `8` | Max concurrent block fetch tasks (IO-bound) |
| `--max-api-range` | `MAX_API_RANGE` | `1000` | Max block range per `/roots` API request |
| `--stream-timeout-secs` | `STREAM_TIMEOUT_SECS` | `180` | Seconds without a new root before the stream is rebuilt; keep it above the eth client's 130 s block-fetch retry budget so fallbacks get a chance to serve |
| `--sled-db-path` | `SLED_DB_PATH` | `./data/roots.sled` | Path to the sled database directory |
| `--api-bind` | `API_BIND` | `0.0.0.0:8080` | HTTP API bind address |
| `--flush-every` | `FLUSH_EVERY` | `10000` | Catch-up batch size: write roots (and request a flush) every N blocks |
| `--tip-window` | `TIP_WINDOW` | `256` | Within N blocks of the head, write every root immediately; durability flushes throttled to ~1/s |
| `--backfill` | — | `false` | Scan for gaps and fill them before resuming |
| `--head-poll-interval-secs` | `HEAD_POLL_INTERVAL_SECS` | `12` | `eth_blockNumber` poll alongside the `newHeads` subscription; bounds how long a silent subscription can stall archiving |
| `--rpc-timeout-secs` | `RPC_TIMEOUT_SECS` | `30` | Deadline per RPC call while (re)establishing the block stream |
| `--ready-lag-blocks` | `READY_LAG_BLOCKS` | `1000` | `/ready` is 503 when more than this many blocks behind the mature target |
| `--stale-after-secs` | `STALE_AFTER_SECS` | `60` | `/ready` is 503 when the source head has not been sampled for this long |
| `--reanchor-max-depth` | `REANCHOR_MAX_DEPTH` | `0` | Stored blocks the archiver may drop to re-anchor on the canonical chain when the stored tip is on a fork; `0` fails closed |
| `--finalization_lag_override` | - | *(none)* | Configurable finalization lag override |

A `.env` file in the working directory is loaded automatically.

## API Endpoints

The API binds before the source-chain handshake, so `/status` and `/roots*` answer during the
handshake, the anchor check and a long `--backfill`. `/ready` is 503 with
`source chain handshake not complete` until the source identity is verified and pinned.

### `GET /status`

Liveness plus freshness. Always `200` while the store is readable (a store error is `500`), so a
stale archive is visible in the body rather than hidden behind a status code. Ages are
milliseconds; `null` means "never".

```json
{
  "chain_id": 56,
  "finalization_lag": 10,
  "uptime_ms": 86400000,
  "latest_archived_block": 1234567,
  "total_blocks": 1234568,
  "source_head": 1234580,
  "source_head_age_ms": 2100,
  "mature_target": 1234570,
  "lag_blocks": 3,
  "last_progress_age_ms": 900,
  "last_flush_ok_age_ms": 400,
  "flush_errors": 0,
  "last_flush_error": null,
  "reconnects": 2,
  "ready": true,
  "not_ready_reasons": []
}
```

### `GET /ready`

Same body as `/status`; `200` only when the source head was sampled within `STALE_AFTER_SECS`,
`lag_blocks <= READY_LAG_BLOCKS`, and the last durability flush succeeded, otherwise `503`.
Point Kubernetes readiness probes and the proof provider's health check here. A halted source
chain with a fresh head sample and full coverage is ready; a silently stale subscription with an
unchanged height is not.

### `GET /roots/latest`

Returns the latest archived block number.

```json
{
  "latest_block": 1234567
}
```

### `GET /roots?from=100&to=200`

Returns merkle roots for an inclusive block range (max `MAX_API_RANGE` blocks per request, default 1,000).

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

1. **StreamRoots** subscribes to new block headers over the WebSocket client and fetches full blocks and receipts over that same client (falling back to `--rpc-fallback-urls` on "not found" / transport errors); the HTTP endpoint is used for chain-head tracking and the canonical-anchor check
2. Blocks are merkleized in parallel using `spawn_blocking` to avoid blocking the async runtime
3. Roots are batched and written to sled in height order
4. On restart, the archiver reads the latest stored height and resumes from there
5. The `--backfill` flag scans for any gaps and fills them before continuing

## Canonical anchor

The reorg guard in the store only fires when an already-stored height is written again with
different content. Resuming after a restart and reconnecting after a stream death both continue
from `stored tip + 1`, so on its own that guard cannot notice a reorg that happened while the
archiver was away (or a finalization lag that was set too small).

Before (re)starting the stream the archiver therefore re-fetches the block at the stored tip and
compares its hash with the one persisted next to the root:

- match → resume from `tip + 1`
- legacy entry without a stored hash → warn, resume (cannot be verified)
- mismatch → the tail sits on an abandoned fork. With the default `--reanchor-max-depth 0` the
  archiver exits with `AnchorMismatch` and leaves the store untouched. With `N > 0` it walks back
  at most `N` stored blocks to the last canonical one, deletes everything above it and resumes
  from there; if no canonical block is found within `N` it still fails closed. The bound is what
  keeps an RPC that serves the wrong chain from wiping the archive.

Note: the chain-id pin (`ChainIdMismatch`) catches an endpoint that serves a *different* chain;
the anchor check catches the *same* chain having moved under us.

## Reconnection

The archiver handles two failure modes:

- **Clean disconnect** — the WS stream returns `None`, triggering immediate reconnection with exponential backoff
- **Stale connection** — the WS stream hangs (server stops sending headers without closing the socket). A 120-second timeout detects this and forces reconnection
