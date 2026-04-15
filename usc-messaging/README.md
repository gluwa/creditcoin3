# USC Messaging

USC Write-Ability

## Full Demonstration Steps
### 0. Install dependencies and Set Environment Vars

- Anvil
- Forge
- npm install

**.env setup**:

Next you'll need to set up your environment variables. This should be straightforward.
For a typical setup with local chains just copy `.env.example`:
```bash
cp .env.example .env
```

TODO: Use Sepolia and Creditcoin3 Testnet as the chains for this demo. Then fund
accounts using faucets.

The private keys in this .env.example are well known dev keys, but you will still have 
to fund the address corresponding to `CREDITCOIN_CHAIN_PRIVATE_KEY` manually:

TODO: How to fund creditcoin evm address on local chain

### 1. Run Local Chains

First we spin up a local anvil destination chain:
```bash
anvil --block-time 6
```

Then we build and launch our local Creditcoin chain:
```bash
cd ..
cargo build --features=fast-runtime --release
./target/release/creditcoin3-node --dev --tmp
```

### 2. Deploy Write-ability Contracts

We want to deploy several contracts in this step:
1. The sample `dApp contract`. This contract lives on Creditcoin and will request to send writability messages
2. The `outbox contract`. This contract lives on Creditcoin and is where message requests are submitted and processed
3. The `inbox contract`. This contract lives on the destination chain. It processes incoming messages from Creditcoin.
4. The `destination contract`. This contract lives on the destination chain. It acts as the endpoint where a dApp was attempting to send its messages.

We have simplified the deployment of these contracts with a single script:
```bash
cd usc-messaging/contracts
./script/deploy.sh
```

This script also saves the addresses of all deployed contracts in `.env` for
later use.

### 3. Run mock Attestor, Relayer, and DApp Message Acknowledgement worker
First start the attestor:
```bash
cd ..
npm run dev:attester
```

Then start the relayer:
```bash
npm run dev:relayer
```

```bash
npx tsx scripts/dApp-ack-worker.ts
```

### 4. Submit message request to dApp contract
To submit our message, run the following:
```bash
npx tsx scripts/publish-message.ts
```

### 5. Watch for automated message signing, sending, and acknowledgement

The first component to pick up your message will be the attestor. It will 
detect a `MessagePublished` event and print something like:

```
[Outbox] MessagePublished messageId=0x933df8cd4be30caa6aad59374988f9f4a917f69d4cf56b19f706549d67b5f376 emitter=0x1cf3a2eeead7c152bb79fbbe767669ebfd6fb0b7000000000000000000000000
[Relayer] POST http://127.0.0.1:3301/deliver messageId=0x933df8cd4be30caa6aad59374988f9f4a917f69d4cf56b19f706549d67b5f376
[Relayer] messageId=0x933df8cd4be30caa6aad59374988f9f4a917f69d4cf56b19f706549d67b5f376 successfully notified to relayer
```

Next the relayer will log its message delivery process:

```
[Attester] Received messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 from attester
[Worker] Queued messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 (queue size: 1)
[Worker] Processing 1 pending message(s)
[Inbox] MessageDelivered messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753
[Inbox] messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 did not request ACK, skipping acknowledgment
[Worker] Delivered messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 tx=0x5297dfe7832db6e83446b3c331f39121649f3ba13873a134bc47a804608716b0
```

Then the `dApp-ack-worker` picks up on the `MessageReceived` event emitted by the destination contract.
It it forwards the acknowledgement and logs the process:
```
MessageReceived
  messageId: 0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753
  emitter:   0x767669EbFd6FB0b7000000000000000000000000
  payload:   0x68656c6c6f20777269746162696c697479
  txHash:    0x5297dfe7832db6e83446b3c331f39121649f3ba13873a134bc47a804608716b0
markDelivered tx sent: 0x65ae778f1639d7e1bc27dfd2d0efc28fe9ca1c53520307c5fa4e9cfd91d565f9
markDelivered confirmed in block 7889
```

Finally, our `publish-message` script listens for the `MessageDelivered` event
emitted from our simpleDApp contract on Creditcoin. 
```
⏳ Waiting for MessageDelivered events...
📬 MessageDelivered event received!
🆔 messageId: 0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753
```

These logs demonstrate that our message delivery and acknowldegement by the dApp contract
were successful!


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

The relayer watches `messages.json` and POST `/deliver` for messages. After deploy, `deployments.json` contains `inbox`, `destination`, `relayer` addresses.

### Relayer Config

| Env / CLI | Default | Description |
|-----------|---------|-------------|
| `DESTINATION_CHAIN_RPC_URL` / `--rpc-url` | http://127.0.0.1:8545 | Destination chain RPC |
| `INBOX_ADDR` / `--inbox` | from deployments.json | SimpleInbox address |
| `DESTINATION_CHAIN_PRIVATE_KEY` | (Anvil #1) | Key that pays gas |
| `RELAYER_MESSAGES_FILE` | ./messages.json | Mock P2P messages file |
| `RELAYER_HTTP_PORT` | 3301 | POST /deliver endpoint |
