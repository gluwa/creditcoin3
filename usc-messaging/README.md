# USC Messaging

USC Write-Ability Layer components: **quoter**, relayer client, and related messaging infrastructure.

## Structure

```
usc-messaging/
├── src/
│   ├── quoter/          # Quotation service
│   └── relayer/         # Relayer client (delivers to inbox)
├── contracts/           # Foundry (Solidity)
│   ├── src/             # DummyInbox, DummyRelayerContract, TestDestination, etc.
│   └── script/Deploy.s.sol
├── scripts/
│   ├── deploy.sh
│   └── seed-message.ts
├── deployments.json     # Written by deploy (inbox, destination, relayer addresses)
├── messages.json        # Mock P2P ready messages (relayer consumes)
└── package.json
```

## Quoter

Off-chain service that provides signed fee quotes for cross-chain messaging. The relayer contract accepts these quotes.

### Quick Start

```bash
npm install
npm run dev:quoter
```

With options at startup:

```bash
npm start -- --payee-address 0x1234567890123456789012345678901234567890
npm start -- --rpc-url https://eth.llamarpc.com
npm start -- --rpc-url http://localhost:8545 --payee-address 0x...
```

When `--rpc-url` is set, the quoter fetches gas price from that RPC and derives `chainId` via `eth_chainId`. You can omit `destinationChainId` in quote requests.

Server runs on `http://localhost:3300` (or `QUOTER_PORT`).

Uses **morgan** (request logging), **helmet** (security headers), and **cors**.

### API

**GET /quote**

| Query param          | Required | Description                          |
|----------------------|----------|--------------------------------------|
| `destinationChainId` | Yes      | EVM chain ID (e.g. 31337 for Anvil) |
| `requiresAck`        | Yes      | `true` or `false`                    |
| `gasLimit`           | No       | Custom gas limit for delivery        |

Example:

```
GET /quote?destinationChainId=31337&requiresAck=false
```

Response (JSON):

```json
{
  "relayPrice": "1234567890",
  "acknowledgmentPrice": "0",
  "payeeAddress": "0x...",
  "paymentToken": "0x0000000000000000000000000000000000000000",
  "expiry": 1737123456,
  "signature": "0x..."
}
```

### Configuration

| Env var                    | Default                         | Description                    |
|----------------------------|---------------------------------|--------------------------------|
| `QUOTER_PORT`              | 3300                            | HTTP server port               |
| `QUOTER_SIGNER_PRIVATE_KEY` | (dev key)                      | EOA key for signing quotes     |
| `QUOTER_PAYEE_ADDRESS`    | 0x0...1                         | Relayer pool address           |
| `QUOTER_PAYMENT_TOKEN`    | 0x0                             | address(0) = native currency   |
| `QUOTER_EXPIRY_SECONDS`   | 3600                            | Quote validity                 |
| `QUOTER_DESTINATION_RPC_URL` | -                            | RPC for gas price (optional)   |

**CLI args** (override env): `--payee-address 0x...`, `--payment-token 0x...`, `--rpc-url https://...` (or `-p`, `-t`, `-r`)

### Development

- **Phase 1**: Fixed/dummy exchange rates (config-based)
- **Phase 2**: Real gas price from destination chain RPC
- **Phase 3**: Exchange rate API (Chainlink, etc.)
- **Phase 4**: Core fee, production key management

See `usc-write-ability-research/documents/requirements/03-quotation-requirements.md`.

---

## Relayer Client

Off-chain client that picks up "ready" messages (mock P2P) and delivers them to the DummyInbox.

### Quick Start

```bash
# 1. Deploy contracts (Anvil must be running)
anvil &
npm run deploy

# 2. Seed a sample message
npm run seed-message

# 3. Start relayer (reads deployments.json for inbox address)
npm run dev:relayer
```

The relayer watches `messages.json` and POST `/deliver` for messages. After deploy, `deployments.json` contains `inbox`, `destination`, `relayer` addresses.

### Relayer Config

| Env / CLI | Default | Description |
|-----------|---------|-------------|
| `RELAYER_RPC_URL` / `--rpc-url` | http://127.0.0.1:8545 | Destination chain RPC |
| `RELAYER_INBOX_ADDRESS` / `--inbox` | from deployments.json | DummyInbox address |
| `RELAYER_PRIVATE_KEY` | (Anvil #1) | Key that pays gas |
| `RELAYER_MESSAGES_FILE` | ./messages.json | Mock P2P messages file |
| `RELAYER_HTTP_PORT` | 3301 | POST /deliver endpoint |

---

## PoC Flow (End-to-End)

1. **Start Anvil**: `anvil`
2. **Deploy**: `npm run deploy` → writes `deployments.json`
3. **Start Quoter**: `npm run dev:quoter -- --rpc-url http://127.0.0.1:8545`
4. **Seed message**: `npm run seed-message` → creates `messages.json`
5. **Start Relayer**: `npm run dev:relayer` → delivers to inbox, TestDestination receives

Optional: POST a message directly to the relayer:

```bash
curl -X POST http://localhost:3301/deliver -H "Content-Type: application/json" -d '{
  "messageId": "0x0000000000000000000000000000000000000000000000000000000000000002",
  "emitterAddress": "0x70997970C51812dc3A010C7d01b50e0d17dc79C8",
  "destinationContract": "<from deployments.json>",
  "payloadData": "0x"
}'
```
