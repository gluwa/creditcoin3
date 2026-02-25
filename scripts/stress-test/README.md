# Proof-Gen API Stress Test

Floods the proof-gen API with configurable volumes of valid, invalid, or mixed
requests at controlled rates to test performance under load.

## Modes

- **valid** — Sends real proof requests using block/tx data fetched from the
  source chain
- **invalid** — Sends malformed or semantically wrong requests (bad chain keys,
  non-existent blocks, malformed hashes, etc.)
- **mixed** — Sends a configurable ratio of valid and invalid requests

### Invalid Request Categories

| Category               | Examples                                |
| ---------------------- | --------------------------------------- |
| Wrong chain key        | `chain_key + 1`, `0`, `999999`          |
| Non-existent block     | Block numbers far beyond current height |
| Block before genesis   | Block `0`, block `1`                    |
| Out-of-bounds tx index | `tx_count + 1`, `999999`                |
| Malformed tx hash      | Too short, too long, non-hex characters |
| Non-existent tx hash   | Valid format but random 32 bytes        |
| Invalid path params    | Strings where numbers expected          |

## Usage

```bash
# Invalid mode (no source chain RPC needed)
deno task start -m invalid -a http://localhost:3100 -c 1

# Valid mode
deno task start -m valid -a http://localhost:3100 -c 1 -s https://sepolia.infura.io/v3/KEY

# Mixed mode (70% valid, 30% invalid) at 100 rps for 2 minutes
deno task start -m mixed -a http://localhost:3100 -c 1 -s https://sepolia.infura.io/v3/KEY \
  --mix-ratio 0.7 --rps 100 --duration 120

# See all options
deno task start -h
```

## Options

| Flag               | Env Var          | Description                                       | Default                    |
| ------------------ | ---------------- | ------------------------------------------------- | -------------------------- |
| `-m, --mode`       | `MODE`           | Test mode: `valid`, `invalid`, `mixed`            | (required)                 |
| `-a, --api-url`    | `API_URL`        | Proof-gen API URL                                 | (required)                 |
| `-c, --chain-key`  | `CHAIN_KEY`      | Chain key                                         | (required)                 |
| `-s, --source-rpc` | `SOURCE_RPC_URL` | Source chain HTTP RPC URL                         | (required for valid/mixed) |
| `--rps`            | `RPS`            | Target requests per second                        | `50`                       |
| `--concurrency`    | `CONCURRENCY`    | Max concurrent requests                           | `20`                       |
| `--duration`       | `DURATION`       | Test duration in seconds                          | `60`                       |
| `--mix-ratio`      | `MIX_RATIO`      | Ratio of valid requests in mixed mode (0.0-1.0)   | `0.5`                      |
| `--block-range`    | `BLOCK_RANGE`    | Block range for valid requests (e.g. `1000-2000`) | latest 50 blocks           |

## Output

Live stats every 2 seconds:

```
[12s] Sent: 600 | OK: 480 | Err: 120 | RPS: 50.2 | p50: 45ms | p99: 230ms
```

Final summary:

```
=== Stress Test Complete ===
Duration:     60.0s
Total:        3000 requests
Successful:   2400 (80.0%)
Failed:       600 (20.0%)
Throughput:   50.0 req/s

Latency (ms):
  p50: 42    p90: 120    p95: 180    p99: 310    max: 1200

Status codes:
  200: 2400
  400: 300
  404: 200
  503: 80
  500: 20

Error breakdown:
  InvalidChainKey: 150
  BlockBeforeGenesis: 100
  ...

By request type:
  Valid:   2100 sent, 2400 succeeded (80.0%)
  Invalid: 900 sent
```
