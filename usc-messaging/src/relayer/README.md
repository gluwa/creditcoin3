# Relayer

The relayer bridges voted messages from attesters to the destination chain and closes the ACK loop on the source chain:

- **Delivery worker** — maintains an in-memory queue of messages received from attesters. On each tick it attempts to deliver each pending message to the `SimpleInbox` contract on the destination chain. Failed deliveries are retried on the next tick.
- **ACK worker** — polls the `SimpleInbox` contract for `MessageDelivered` events and calls `Outbox.acknowledgeMessage` on the source chain for each one.

Messages are accepted via HTTP POST and queued immediately — delivery is decoupled from receipt. Duplicate `messageId` values are silently dropped.

## Running

Development (no build step):

```sh
npm run dev:relayer -- --inbox 0xINBOX --outbox 0xOUTBOX --private-key 0xKEY
```

Production (after `npm run build`):

```sh
npm run relayer -- --inbox 0xINBOX --outbox 0xOUTBOX --private-key 0xKEY
```

## Configuration

Options are resolved in order: **CLI arg → environment variable → default**.

| CLI arg                   | Environment variable              | Default                   | Description                                   |
|---------------------------|----------------------------------|---------------------------|-----------------------------------------------|
| `--inbox` / `-i`          | `INBOX_ADDR`          | —                         | SimpleInbox contract address (required)       |
| `--outbox` / `-o`         | `OUTBOX_ADDR`         | —                         | Outbox contract address for ACK (required)    |
| `--private-key` / `-k`    | `DESTINATION_CHAIN_PRIVATE_KEY`            | —                         | Private key for signing txs (required)        |
| `--rpc-url` / `-r`        | `DESTINATION_CHAIN_RPC_URL`                | `http://127.0.0.1:8545`   | Destination chain RPC endpoint                |
| `--source-rpc-url`        | `CREDITCOIN_RPC_URL`         | `http://127.0.0.1:9944`   | Source chain RPC endpoint (for ACK)           |
| `--delivery-interval`     | `RELAYER_DELIVERY_INTERVAL_MS`   | `5000`                    | Worker tick interval in milliseconds          |
| `--http-port`             | `RELAYER_HTTP_PORT`              | `3301`                    | HTTP port for POST /deliver (0 = disabled)    |

### `deployments.json` fallback

If `--inbox` or `--outbox` are not provided via CLI or env vars, the relayer looks for a `deployments.json` file in the working directory:

```json
{
  "inbox": "0x...",
  "outbox": "0x..."
}
```

## API

**POST /deliver**

Accepts a voted message from an attester and adds it to the delivery queue.

```json
{
  "messageId":      "0x...",
  "emitterAddress": "0x...",
  "payload":        "0x...",
  "requiresAck":    true,
  "signedVotes":    ["0x..."]
}
```

Returns `202` in all cases:
- `{ "queued": true, "messageId": "0x..." }` — message accepted
- `{ "queued": false, "reason": "duplicate" }` — messageId already pending

**GET /health**

```json
{ "status": "ok", "pending": 3 }
```

## Architecture

```
Attester                    Relayer (destination chain)       Source chain (Outbox)
  POST /deliver  ──────►  pendingQueue
                              │
                    [delivery worker tick]
                              │
                     SimpleInbox.deliverMessage
                              │
                     MessageDelivered event
                              │
                    [ACK worker poll]
                              │
                     Outbox.acknowledgeMessage  ──────►  MessageAcknowledged
```

Both workers use `queryFilter` polling (no WebSocket needed) and start from the latest block at startup — events emitted before the relayer starts are not replayed.
