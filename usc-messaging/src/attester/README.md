# Attester

The attester watches the source chain and bridges events to the relayer:

- **Source chain** — polls the `Outbox` contract for `MessagePublished` events, signs each message ID with its private key (the "vote"), then forwards the signed message to the relayer via HTTP POST.

## Running

Development (no build step):

```sh
npm run dev:attester -- --outbox 0xOUTBOX --private-key 0xKEY
```

Production (after `npm run build`):

```sh
npm run attester -- --outbox 0xOUTBOX --private-key 0xKEY
```

## Configuration

Options are resolved in order: **CLI arg → environment variable → default**.

| CLI arg                 | Environment variable           | Default                   | Description                              |
|-------------------------|-------------------------------|---------------------------|------------------------------------------|
| `--outbox` / `-o`       | `ATTESTER_OUTBOX_ADDRESS`     | —                         | Outbox contract address (required)       |
| `--private-key` / `-k`  | `RELAYER_PRIVATE_KEY`         | —                         | Private key used to sign messages (required) |
| `--source-rpc-url`      | `ATTESTER_SOURCE_RPC_URL`     | `http://127.0.0.1:9944`   | Source chain RPC endpoint                |
| `--relayer-url`         | `ATTESTER_RELAYER_URL`        | `http://127.0.0.1:3301`   | Relayer HTTP base URL                    |
| `--poll-interval`       | `ATTESTER_POLL_INTERVAL_MS`   | `5000`                    | Event polling interval in milliseconds   |

### `deployments.json` fallback

If `--outbox` is not provided via CLI or env vars, the attester looks for a `deployments.json` file in the working directory:

```json
{
  "outbox": "0x..."
}
```

## Architecture

```
Source chain (Outbox)          Attester                          Relayer
  MessagePublished  ──poll──►  sign(messageId) → notifyRelayer  ──POST /deliver──►  relayer
```

The listener uses `queryFilter` polling (no WebSocket needed) and starts from the latest block at startup — messages published before the attester starts are not replayed.
