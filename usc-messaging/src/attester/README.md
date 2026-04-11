# Attester

The attester watches both chains and bridges events between them:

- **Source chain** — polls the `Outbox` contract for `MessagePublished` events, then forwards each message to the relayer via HTTP POST.
- **Destination chain** — polls the `Inbox` contract for `MessageDelivered` events and logs delivery (ACK flow is not yet implemented).

In the current POC the attester's "vote" is implicit: receiving a `MessagePublished` event is treated as approval and the message is immediately forwarded to the relayer.

## Running

Development (no build step):

```sh
npm run dev:attester -- --outbox 0xOUTBOX --inbox 0xINBOX
```

Production (after `npm run build`):

```sh
npm run attester -- --outbox 0xOUTBOX --inbox 0xINBOX
```

## Configuration

Options are resolved in order: **CLI arg → environment variable → default**.

| CLI arg             | Environment variable           | Default                   | Description                              |
|---------------------|-------------------------------|---------------------------|------------------------------------------|
| `--outbox` / `-o`   | `ATTESTER_OUTBOX_ADDRESS`     | —                         | Outbox contract address (required)       |
| `--inbox` / `-i`    | `ATTESTER_INBOX_ADDRESS`      | —                         | Inbox contract address (required)        |
| `--source-rpc-url`  | `ATTESTER_SOURCE_RPC_URL`     | `http://127.0.0.1:9944`   | Source chain RPC endpoint                |
| `--destination-rpc-url` | `ATTESTER_DESTINATION_RPC_URL` | `http://127.0.0.1:8545` | Destination chain RPC endpoint           |
| `--relayer-url`     | `ATTESTER_RELAYER_URL`        | `http://127.0.0.1:3301`   | Relayer HTTP base URL                    |
| `--poll-interval`   | `ATTESTER_POLL_INTERVAL_MS`   | `5000`                    | Event polling interval in milliseconds   |

### `deployments.json` fallback

If `--outbox` / `--inbox` are not provided via CLI or env vars, the attester looks for a `deployments.json` file in the working directory:

```json
{
  "outbox": "0x...",
  "inbox": "0x..."
}
```

## Architecture

```
Source chain (Outbox)          Attester              Relayer
  MessagePublished  ──poll──►  notifyRelayer  ──POST /deliver──►  relayer

Destination chain (Inbox)
  MessageDelivered  ──poll──►  log (ACK: TODO)
```
/ex
The listener uses `queryFilter` polling (no WebSocket needed) and starts from the latest block at startup — messages published before the attester starts are not replayed.
