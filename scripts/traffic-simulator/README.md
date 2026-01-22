# Proof Traffic Simulator

A Deno-based TypeScript tool that simulates proof query traffic for Creditcoin3-next by:

1. **Streaming blocks** from source chain (Sepolia) via WebSocket
2. **Queueing blocks** until they are attested on Creditcoin3
3. **Submitting proofs** for random transactions once blocks are attested
   - Uses `/api/v1/proof/{chain_key}/{header_number}/{tx_index}` by default (falls back to `/proof-by-tx`)

## Requirements

- [Deno](https://deno.land/) 2.x
- Access to Sepolia WebSocket RPC (e.g., Infura, Alchemy)
- Running Creditcoin3-next node
- Running Proof Generation API server

## Quick Start

```bash
# Development with hot reload
deno task dev -- \
  --source-rpc wss://sepolia.infura.io/ws/v3/YOUR_KEY \
  --private-key 0x...

# Production run
deno task start -- \
  --source-rpc wss://sepolia.infura.io/ws/v3/YOUR_KEY \
  --cc3-ws wss://rpc.ccnext.creditcoin.network \
  --private-key 0x... \
  --api-url http://proof-gen-api:3100
```

## Configuration

### CLI Arguments

| Argument | Description | Default |
|----------|-------------|---------|
| `--source-rpc` | Source chain WebSocket RPC URL | Required |
| `--cc3-ws` | Creditcoin3 WebSocket URL | `ws://localhost:9944` |
| `--cc3-http` | Creditcoin3 HTTP URL | Derived from WS |
| `--private-key` | Private key for signing | Required |
| `--api-url` | Proof generation API URL | `http://localhost:3100` |
| `--query-mode` | Query complexity mode | `transfer` |
| `--chain-key` | Source chain key (Sepolia: 1) | `1` |
| `--max-queue-size` | Max blocks to track in queue | `100` |
| `--tx-per-block` | Max txs per block (random 1..N) | `10` |
| `--batch-size` | Max batch size (random 1..N, max 10) | `10` |
| `--batch-probability` | Probability of batch mode | `0.3` |
| `--health-port` | Health check port | `8080` |
| `--verbose` | Enable verbose debug logging | `false` |
| `--enable-query-builder` | Enable query builder logging | `true` |
| `--disable-query-builder` | Disable query builder logging | `false` |

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `SOURCE_RPC_URL` | Source chain WebSocket RPC | - |
| `CC3_WS_URL` | Creditcoin3 WebSocket URL | `ws://localhost:9944` |
| `CC3_HTTP_URL` | Creditcoin3 HTTP URL | Derived from WS |
| `CC3_PRIVATE_KEY` | Private key for signing | - |
| `PROOF_API_URL` | Proof generation API | `http://localhost:3100` |
| `CHAIN_KEY` | Source chain key (1=Sepolia on testnet) | `1` |
| `TX_PER_BLOCK` | Max txs per block (random 1..N) | `10` |
| `BATCH_SIZE` | Max batch size (random 1..N, max 10) | `10` |
| `BATCH_PROBABILITY` | Probability of batch mode | `0.3` |
| `QUERY_MODE` | Query complexity mode | `transfer` |
| `ENABLE_QUERY_BUILDER` | Build/log query layouts | `true` |
| `LOG_VERBOSE` | Enable verbose debug logging | `false` |
| `HEALTH_PORT` | Health check port | `8080` |

### Query Modes

| Mode | Fields Included | Use Case |
|------|----------------|----------|
| `minimal` | RxStatus only | Basic proof verification |
| `transfer` | From, To, Value, Status | Native token transfers |
| `full` | All transaction fields | Complete transaction data |
| `erc20` | Transfer event data | ERC20 token transfers |

## Health Endpoints

The simulator exposes health check endpoints on port 8080:

- `GET /health` - Liveness probe (always returns 200 if running)
- `GET /ready` - Readiness probe (returns 200 if connected to both chains)
- `GET /metrics` - Prometheus metrics
- `GET /status` - Detailed status JSON

## Docker

### Build

```bash
docker build -t gluwa/proof-traffic-simulator:latest .
```

### Run

```bash
docker run -d \
  -e SOURCE_RPC_URL=wss://sepolia.infura.io/ws/v3/YOUR_KEY \
  -e CC3_WS_URL=wss://rpc.ccnext.creditcoin.network \
  -e CC3_PRIVATE_KEY=0x... \
  -e PROOF_API_URL=http://proof-gen-api:3100 \
  -p 8080:8080 \
  gluwa/proof-traffic-simulator:latest
```

## Kubernetes

```bash
# Create secrets (edit with real values first)
cp k8s/secrets.yaml.example k8s/secrets.yaml
# Edit k8s/secrets.yaml with your actual values
kubectl apply -f k8s/secrets.yaml

# Deploy
kubectl apply -f k8s/deployment.yaml
```

## Development

```bash
# Run with hot reload
deno task dev -- --source-rpc wss://... --private-key 0x...

# Format code
deno task fmt

# Lint
deno task lint

# Type check
deno task check

# Run tests
deno task test
```

## Compile to Binary

```bash
# Compile to a single executable
deno task compile

# Run the compiled binary
./simulator --source-rpc wss://... --private-key 0x...
```

## cc-next-query-builder Integration

This simulator uses the [`@gluwa/cc-next-query-builder`](https://github.com/gluwa/cc-next-query-builder) SDK to build structured queries for proof verification. You can disable query layout logging with `ENABLE_QUERY_BUILDER=false` or `--disable-query-builder`.

### What the SDK Does

The query builder helps compose **layout segments** that define which parts of a transaction/receipt should be included in the proof query:

- **Static fields**: Transaction fields like `TxFrom`, `TxTo`, `TxValue`, receipt fields like `RxStatus`
- **Function arguments**: Decode and include specific calldata fields
- **Event data**: Include ERC20 Transfer events or other logged events

### Query Modes

| Mode | SDK Fields Used | Description |
|------|-----------------|-------------|
| `minimal` | `RxStatus` | Only verifies transaction success |
| `transfer` | `TxFrom`, `TxTo`, `TxValue`, `RxStatus` | Native ETH transfers |
| `full` | All static fields | Complete transaction verification |
| `erc20` | Transfer event + `RxStatus` | ERC20 token transfers with events |

### Layout Segments

When submitting proofs, the simulator logs the computed layout segments:

```
📐 Layout segments: [0:32], [64:96], [128:160]
```

These offsets define which bytes of the RLP-encoded transaction/receipt are being verified.

## Architecture

```
┌─────────────────┐     ┌─────────────────┐
│  Sepolia Node   │     │  Creditcoin3    │
│  (WebSocket)    │     │  (WebSocket)    │
└────────┬────────┘     └────────┬────────┘
         │                       │
         ▼                       ▼
┌─────────────────┐     ┌─────────────────┐
│ BlockSubscriber │     │  Attestation    │
│                 │     │  Subscriber     │
└────────┬────────┘     └────────┬────────┘
         │                       │
         ▼                       ▼
┌─────────────────────────────────────────┐
│            Pending Block Queue          │
└────────────────────┬────────────────────┘
                     │
                     ▼ (on attestation)
┌─────────────────────────────────────────┐
│          Query Factory                  │
│   (cc-next-query-builder SDK)           │
└────────────────────┬────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────┐
│          Proof Submitter                │
│   ┌─────────────┐  ┌─────────────┐      │
│   │   Single    │  │    Batch    │      │
│   │  Submitter  │  │  Submitter  │      │
│   └──────┬──────┘  └──────┬──────┘      │
└──────────┼────────────────┼─────────────┘
           │                │
           ▼                ▼
┌─────────────────────────────────────────┐
│       Proof Gen API Server              │
└────────────────────┬────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────┐
│       Block Prover Precompile           │
└─────────────────────────────────────────┘
```

## Attestation Lag

The simulator only submits proofs for blocks **strictly less** than the latest
attested block. The newest attested block becomes provable after the next
attestation event arrives. This avoids `Continuity proof does not match
attestation or checkpoint` errors caused by proving the current attested block.

## Metrics

Prometheus metrics available at `/metrics`:

| Metric | Type | Description |
|--------|------|-------------|
| `simulator_blocks_queued_total` | Counter | Total blocks added to queue |
| `simulator_blocks_processed_total` | Counter | Total blocks processed |
| `simulator_proofs_submitted_total` | Counter | Total proofs submitted |
| `simulator_single_submissions_total` | Counter | Single proof submissions |
| `simulator_batch_submissions_total` | Counter | Batch proof submissions |
| `simulator_proof_errors_total` | Counter | Proof submission errors |
| `simulator_queue_size` | Gauge | Current queue size |
| `simulator_sepolia_connected` | Gauge | Sepolia connection status |
| `simulator_cc3_connected` | Gauge | CC3 connection status |
