# Proof Traffic Simulator

A Deno-based TypeScript tool that simulates proof query traffic for
Creditcoin3-next by:

1. **Streaming blocks** from source chain (Sepolia) via WebSocket
2. **Queueing blocks** until they are attested on Creditcoin3
3. **Submitting proofs** for random transactions once blocks are attested
   - Uses `/api/v1/proof/{chain_key}/{header_number}/{tx_index}` by default
     (falls back to `/proof-by-tx`)

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
  --cc3-ws wss://rpc.usc-testnet2.creditcoin.network \
  --private-key 0x... \
  --api-url https://proof-gen-api.usc-testnet2.creditcoin.network
```

## Configuration

### CLI Arguments

| Argument              | Description                               | Default                 |
| --------------------- | ----------------------------------------- | ----------------------- |
| `--source-rpc`        | Source chain WebSocket RPC URL            | Required                |
| `--cc3-ws`            | Creditcoin3 WebSocket URL                 | `ws://localhost:9944`   |
| `--cc3-http`          | Creditcoin3 HTTP URL                      | Derived from WS         |
| `--private-key`       | Private key for signing                   | Required                |
| `--api-url`           | Proof generation API URL                  | `http://localhost:3100` |
| `--chain-key`         | Source chain key (Sepolia: 1)             | `1`                     |
| `--max-queue-size`    | Max blocks to track in queue              | `100`                   |
| `--batch-size`        | Max batch size (random 1..N, max 10)      | `10`                    |
| `--batch-probability` | Probability of batch mode                 | `0.5`                   |
| `--single-every`      | Submit a single proof once every N blocks | `1`                     |
| `--health-port`       | Health check port                         | `8080`                  |
| `--verbose`           | Enable verbose debug logging              | `false`                 |

### Environment Variables

| Variable              | Description                               | Default                 |
| --------------------- | ----------------------------------------- | ----------------------- |
| `SOURCE_RPC_URL`      | Source chain WebSocket RPC                | -                       |
| `CC3_WS_URL`          | Creditcoin3 WebSocket URL                 | `ws://localhost:9944`   |
| `CC3_HTTP_URL`        | Creditcoin3 HTTP URL                      | Derived from WS         |
| `CC3_PRIVATE_KEY`     | Private key for signing                   | -                       |
| `PROOF_API_URL`       | Proof generation API                      | `http://localhost:3100` |
| `CHAIN_KEY`           | Source chain key (1=Sepolia on testnet)   | `1`                     |
| `BATCH_SIZE`          | Max batch size (random 1..N, max 10)      | `10`                    |
| `BATCH_PROBABILITY`   | Probability of batch mode                 | `0.5`                   |
| `SINGLE_EVERY_BLOCKS` | Submit a single proof once every N blocks | `1`                     |
| `LOG_VERBOSE`         | Enable verbose debug logging              | `false`                 |
| `HEALTH_PORT`         | Health check port                         | `8080`                  |

Single submissions pick one random transaction once every `SINGLE_EVERY_BLOCKS`.
Batch submissions pick one random transaction per block and group
1..`BATCH_SIZE` blocks into a batch when they share a continuity proof.

## Health Endpoints

The simulator exposes health check endpoints on port 8080:

- `GET /health` - Liveness probe (always returns 200 if running)
- `GET /ready` - Readiness probe (returns 200 if connected to both chains)
- `GET /metrics` - Prometheus metrics
- `GET /status` - Detailed status JSON

## Docker

### Build

Currently can only be built from the root of the repository:

```bash
docker build -f scripts/traffic-simulator/Dockerfile -t gluwa/proof-traffic-simulator .
```

### Run

```bash
docker run -d \
  -e SOURCE_RPC_URL=wss://sepolia.infura.io/ws/v3/YOUR_KEY \
  -e CC3_WS_URL=wss://rpc.usc-testnet2.creditcoin.network\
  -e CC3_PRIVATE_KEY=0x... \
  -e PROOF_API_URL=https://proof-gen-api.usc-testnet2.creditcoin.network \
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

## Slack Notifications

Slack reporting is handled by a separate Kubernetes CronJob that queries the
simulator's `/status` endpoint and sends periodic reports. This approach:

- Separates concerns (simulator vs monitoring)
- Allows independent scaling and scheduling
- Works with multiple simulator instances
- Persists metric snapshots across pod restarts

### Setup Steps

1. **Create Slack Webhook**:
   - Go to https://api.slack.com/apps
   - Create a new app or use existing
   - Navigate to "Incoming Webhooks"
   - Create a webhook for your channel
   - Copy the webhook URL

2. **Get Slack User/Group ID** (optional, for alerts):
   - User ID: Found in user profile URL (e.g., `U123456`)
   - Group ID: Found in group settings (e.g., `S123456`)

3. **Deploy CronJob**:

   ```bash
   # Edit the manifest with your values
   # Update SLACK_WEBHOOK_URL and optionally SLACK_ALERT_GROUP in k8s/cronjob-reporter.yaml

   # Deploy (non-persistent version - snapshot resets on pod restart)
   kubectl apply -f k8s/cronjob-reporter.yaml

   # OR deploy persistent version (snapshot persists across pod restarts)
   kubectl apply -f k8s/cronjob-reporter-persistent.yaml
   ```

4. **Verify**:

   ```bash
   # Check CronJob status
   kubectl get cronjob traffic-simulator-reporter

   # View recent jobs
   kubectl get jobs -l app=traffic-simulator-reporter

   # Check logs of latest job
   kubectl logs -l app=traffic-simulator-reporter --tail=50
   ```

### CronJob Configuration

The CronJob runs every hour at minute 0 (e.g., 1:00, 2:00, 3:00). To change the
schedule, edit the `schedule` field in the manifest:

```yaml
spec:
  schedule: "0 * * * *" # Every hour
  # schedule: "0 */2 * * *"  # Every 2 hours
  # schedule: "0 9-17 * * *"  # Every hour during business hours
```

### Report Format

Hourly reports include:

- **Period**: Start and end timestamps
- **Connection Status**: Sepolia and CC3 connection health
- **Proof Submissions**: Successful/failed counts and success rate
- **Submission Breakdown**: Single vs batch submissions
- **Blocks**: Processed count and current queue size
- **Totals**: Cumulative metrics since simulator start
- **Errors**: Last error message if any

Reports automatically mention the configured alert group if there are proof
errors.

### Troubleshooting

- **No reports received**: Check CronJob logs and ensure webhook URL is correct
- **Missing metrics**: Ensure simulator service is accessible from CronJob pod
- **Snapshot resets**: Use persistent version if you need metrics history across
  pod restarts

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
attestation event arrives. This avoids
`Continuity proof does not match attestation or checkpoint` errors caused by
proving the current attested block.

## Metrics

Prometheus metrics available at `/metrics`:

| Metric                               | Type    | Description                 |
| ------------------------------------ | ------- | --------------------------- |
| `simulator_blocks_queued_total`      | Counter | Total blocks added to queue |
| `simulator_blocks_processed_total`   | Counter | Total blocks processed      |
| `simulator_proofs_submitted_total`   | Counter | Total proofs submitted      |
| `simulator_single_submissions_total` | Counter | Single proof submissions    |
| `simulator_batch_submissions_total`  | Counter | Batch proof submissions     |
| `simulator_proof_errors_total`       | Counter | Proof submission errors     |
| `simulator_queue_size`               | Gauge   | Current queue size          |
| `simulator_sepolia_connected`        | Gauge   | Sepolia connection status   |
| `simulator_cc3_connected`            | Gauge   | CC3 connection status       |
