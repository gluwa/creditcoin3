# USC Messaging

USC Write-Ability

> **Note — attestor & relayer are now in Rust.** The TypeScript mock **attestor** and **relayer**
> workers that used to live here (`src/attestor/`, `src/relayer/`) have been removed: the attestor
> is now the attestor's `tasks/write_ability` module (`attestor/attestor/`), and the relayer is the
> `message-relayer` crate. What remains in this package is the still-TS-only **quoter**, the
> **dApp ack worker**, the Solidity **contracts**, and the demo/deploy scripts. The end-to-end demo
> steps below that invoked `npm run dev:attestor` / `dev:relayer` therefore now point at the Rust
> components instead.

## Full Demonstration Steps

### 0. Prerequisite — Creditcoin chain and anvil

This demo runs on top of the standard local stack described in
[`.github/CONTRIBUTING.md`](../.github/CONTRIBUTING.md). **Complete steps 0–2 of that guide first**
(build, Creditcoin `--dev` node, anvil). **Do not start the attestor zombienet yet** — that is
CONTRIBUTING step 3, which we run in step 3 below, *after* deploying.

### 1. Set environment variables

Install this package's Node dependencies (provides `tsx` and the `@polkadot/api` used by the deploy
script to register the factory on-chain), then copy `.env.example`:
```bash
cd usc-messaging
npm install
cp .env.example .env
```

The private keys in `.env.example` are well-known dev keys, and the defaults already match the local
stack above (`CREDITCOIN_RPC_URL=http://127.0.0.1:9944`,
`DESTINATION_CHAIN_RPC_URL=http://127.0.0.1:8545`, sudo `//Alice`, `chain_key 2`, EVM chain id `42`).

TODO: Offer option to use Sepolia and Creditcoin3 Testnet as the chains for this demo, funding accounts via faucets.

### 2. Deploy Write-ability Contracts

We want to deploy several contracts in this step:
1. The sample `dApp contract`. This contract lives on Creditcoin and will request to send writability messages
2. The `relayer contract`. This contract lives on Creditcoin and processes quotes + payments for messages.
3. The `outbox factory contract`. This contract lives on Creditcoin. It is USC-operated and creates one
  `outbox contract` per destination chain, passing its own owner to each outbox so the same account controls both.
4. The `outbox contract`. This contract lives on Creditcoin and is where message requests are submitted and
  processed. **It is not deployed directly** — the outbox factory creates it (see below).
5. The `inbox contract`. This contract lives on the destination chain. It processes incoming messages from Creditcoin.
6. The `vote validator contract` (**`EOAValidator`**). This contract lives on the destination chain and validates
  attestor votes on messages forwarded from the inbox: it `ecrecover`s each ECDSA signature, requires each signer to
  be a registered attestor, and enforces the 2N/3+1 threshold. It replaces the old always-accept `DummyVoteValidator`.
  It is seeded with a best-effort attestor set at deploy and then synced to the live attestors by
  `launch-attestors.sh` in step 3 (the destination deployer is the validator admin).
7. The `destination contract`. This contract lives on the destination chain. It acts as the endpoint where a dApp was attempting to send its messages.

The outbox follows a **"create factory first → use factory to create outbox"** pattern: the deploy
script deploys the `OutboxFactory`, then calls `createOutbox(chainKey, validator)` on it to create
the chain's outbox, and reads the resulting address back via `getOutbox(chainKey)`. The dApp is then
wired to that factory-created outbox.

> Note: this is the **source-chain `validator` passed into `createOutbox`** (for the outbox's
> `acknowledgeMessage`), which is still a `DummyVoteValidator` placeholder — `acknowledgeMessage`
> remains permissionless. Proper validator-gated acknowledgment access control is a TODO (see the
> comments in `SimpleOutbox.sol`). This is distinct from the **destination-chain vote validator**
> (contract 6 above), which is now the real `EOAValidator`.

We have simplified the deployment of these contracts with a single script:
```bash
cd usc-messaging
npx tsx scripts/deploy.ts
```

After creating the outbox factory and outbox, the script also **registers the factory on-chain** by
submitting `supportedChains.setOutboxFactoryAddr(chainKey, factoryAddress)` to Creditcoin. This is
required so the real (Rust) attestor and relayer can resolve the outbox on-chain via the chain-info
precompile (`outbox_factory_address` → factory → `getOutbox(chainKey)`); the previous dummy
attestor/relayer skipped this. Notes:

- It is a **Substrate extrinsic** (not an EVM tx) and is operator-gated. On a `--dev` node the
  script submits it via **sudo** using `//Alice` (override with `CREDITCOIN_SUDO_SURI`). It connects
  over the Substrate WS RPC (`CREDITCOIN_SUBSTRATE_WS_URL`, defaulting to `CREDITCOIN_RPC_URL` with a
  `ws://` scheme).
- The chain (`chainKey`) must already be a **supported chain**. The dev genesis seeds `chain_key = 2`
  (the local anvil, `Anvil1`) with a zero-address factory placeholder, which this step overwrites with
  the real factory; otherwise the extrinsic reverts with `ChainNotSupported`.

This script also saves the addresses of all deployed contracts in `.env` for
later use (including `OUTBOX_FACTORY_ADDR` for the factory and `OUTBOX_ADDR` for the outbox it created).

### 3. Start the attestors, then the Relayer, Quoter, and dApp acknowledgement worker

Now that the factory and Outbox are registered on-chain (step 2), start the attestors. Use the
helper script below — it launches the attestor zombienet (CONTRIBUTING step 3), discovers each
attestor's derived message-vote EVM address from its logs, writes that set into
`attestor/config.yaml` (`write_ability.attestors`), prints/saves the matching `--attestor-set`
value for the relayer, and **syncs the destination-chain `EOAValidator` to that live set** (via
`updateAttestorSet`, so `deliverMessage` accepts exactly these attestors). It then stays in the
foreground streaming the zombienet logs, like running the zombienet directly (Ctrl-C to stop):

```bash
bash usc-messaging/scripts/launch-attestors.sh        # add a number to run N != 3 attestors
```

> Run this **after** the deploy in step 2. The attestors only log a signer address once write-ability
> resolves the Outbox on-chain; if no factory/Outbox is registered for `chain_key 2` yet, write-ability
> stays disabled and the script times out. (The script uses random — but deterministic — attestor
> keys, not `--well-known-keys`, so the addresses it writes stay valid across restarts.)

When the script prints the attestor set, copy it (also saved to `usc-messaging/scripts/.attestor-set`)
and, in a **separate terminal**, start the relayer. The relayer is the Rust `message-relayer` crate;
it snoops the attestors' `{chain_key}/message-votes/v1` votes, aggregates a 2N/3+1 quorum, and calls
`Inbox.deliverMessage` on the destination anvil chain:

```bash
cd usc-messaging
source .env                                  # exports $OUTBOX_ADDR and $INBOX_ADDR
ATTESTOR_SET=$(cat scripts/.attestor-set)    # the set written by launch-attestors.sh

cargo run -p message-relayer -- --single-route \
  --cc3-rpc-url ws://localhost:9944 \
  --creditcoin-eth-rpc-url http://localhost:9944 \
  --chain-key 2 \
  --cc3-chain-id 42 \
  --outbox-address "$OUTBOX_ADDR" \
  --destination-rpc-url http://localhost:8545 \
  --inbox-address "$INBOX_ADDR" \
  --signer-key 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 \
  --attestor-set "$ATTESTOR_SET"
```

Notes on the relayer flags:
- `--chain-key 2` / `--cc3-chain-id 42` match the dev anvil chain key and the Creditcoin `--dev` EVM
  chain id; both are bound into `messageHash`, so they must agree with the attestors.
- `--attestor-set` is the value produced by `launch-attestors.sh`; the relayer drops votes from
  signers outside it.
- `--signer-key` is anvil's default account 0 — it pays for `deliverMessage` on the destination.

Then start the Quoter (still TS):
```bash
cd usc-messaging
npm run dev:quoter
```

Finally, start the dApp's acknowledgement worker (still TS):
```bash
cd usc-messaging
npx tsx src/dApp-ack-worker/dApp-ack-worker.ts
```

### 4. Submit message request to dApp contract
To submit our message, run the following:
```bash
cd usc-messaging
npx tsx scripts/publish-message/publish-message.ts
```

### 5. Watch for automated message signing, sending, and acknowledgement

1. The first component to pick up your message will be the attestor. It will
detect a `MessagePublished` event and print something like:

```
[Outbox] MessagePublished messageId=0x933df8cd4be30caa6aad59374988f9f4a917f69d4cf56b19f706549d67b5f376 emitter=0x1cf3a2eeead7c152bb79fbbe767669ebfd6fb0b7000000000000000000000000
[Relayer] POST http://127.0.0.1:3301/deliver messageId=0x933df8cd4be30caa6aad59374988f9f4a917f69d4cf56b19f706549d67b5f376
[Relayer] messageId=0x933df8cd4be30caa6aad59374988f9f4a917f69d4cf56b19f706549d67b5f376 successfully notified to relayer
```

2. Next the attestor notifies the relayer, which logs its message delivery process:

```
[Attestor] Received messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 from attestor
[Worker] Queued messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 (queue size: 1)
[Worker] Processing 1 pending message(s)
[Inbox] MessageDelivered messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753
[Inbox] messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 did not request ACK, skipping acknowledgment
[Worker] Delivered messageId=0xd9f28e1ceb013ba9e121f1f10f9f6b9b86e3f4825ab069813ee0c039aaa2f753 tx=0x5297dfe7832db6e83446b3c331f39121649f3ba13873a134bc47a804608716b0
```

3. The inbox contract forwards the message to its designated destination contract. The destination
contract emits a `MessageReceived` event.

4. Then the `dApp-ack-worker` picks up on the `MessageReceived` event emitted by the destination contract.
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

5. Finally, our `publish-message` script listens for the `MessageDelivered` event
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
| `QUOTER_PAYMENT_TOKEN`    | 0x0                             | address(0) = native currency   |
| `QUOTER_EXPIRY_SECONDS`   | 3600                            | Quote validity                 |

**CLI args** (override env): `--payee-address 0x...`, `--payment-token 0x...`, `--rpc-url https://...` (or `-p`, `-t`, `-r`)

### Development

- **Phase 1**: Fixed/dummy exchange rates (config-based)
- **Phase 2**: Real gas price from destination chain RPC
- **Phase 3**: Exchange rate API (Chainlink, etc.)
- **Phase 4**: Core fee, production key management

See `usc-write-ability-research/documents/requirements/03-quotation-requirements.md`.

---

## Relayer (Rust)

The relayer is the `message-relayer` crate (workspace root), not part of this package. It watches the
Creditcoin L1 Outbox for `MessagePublished`, snoops attestor votes on the
`{chain_key}/message-votes/v1` gossip topic, aggregates 2N/3+1, and calls `Inbox.deliverMessage`.
See `message-relayer/README`/`PLAN.md` and `message-relayer/config.example.yaml` for configuration.
