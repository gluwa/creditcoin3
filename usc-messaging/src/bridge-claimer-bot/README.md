# Bridge Claimer Bot

Watches each configured spoke chain's `BridgeVault` for `Deposited` events, waits for the deposit
to be attested, fetches the proof, and submits `BridgeHub.claim` on Creditcoin — so a user never
needs a Creditcoin wallet or gas to receive a bridged transfer. It's a convenience, not a trust
boundary: `BridgeHub.claim` is fully permissionless and idempotent (`claimed` mapping), so anyone
(a user, a script, another instance of this bot) racing this bot to claim the same deposit is a
safe no-op, not a conflict.

See the write-ability bridge POC design for how this fits into the full system (contracts, dApp,
deploy scripts) — this README only covers the bot.

## How it works

1. **`watcher.ts`** polls each route's `BridgeVault` for `Deposited` logs, holding back
   `confirmationDepth` blocks from the chain tip before treating a log as final enough to act on.
2. For each new deposit, **`pending.ts`**'s `ClaimTracker` skips it if it's already claimed,
   in flight, or in backoff (in-memory only — see "Known limitations" below).
3. **`claim.ts`**'s `claimDeposit()` does the actual work:
   - `ProofBuilder.waitUntilHeightAttested` (from `@gluwa/usc-sdk`) waits for the deposit's block
     to be attested.
   - Fetches the proof and assembles the `InclusionProof`/`ContinuityProof` `BridgeHub.claim`
     expects (same encoding `scripts/claim-delivery.mts` already proves out for the relayer's
     fee-claim path).
   - Submits `BridgeHub.claim(sourceChainKey, blockHeight, inclusionProof, continuityProof)`.
4. **`index.ts`** wires this up per configured route into one long-running daemon process, and
   handles `SIGINT` by stopping every route's watcher cleanly.

## Prerequisites

- A deployed `BridgeHub` (Creditcoin) and at least one deployed `BridgeVault` spoke — see
  `usc-messaging/scripts/deploy-bridge-hub.mts`/`deploy-bridge-spoke.mts`, or the
  `usc-bridge-dapp` README's "Deploying the contracts" section.
- A running `proof-gen-api-server` reachable at the URL you'll set as `PROOF_GEN_URL`.
- A Creditcoin EOA with enough gas to submit `claim` transactions (`CLAIM_SIGNER_KEY`). It needs
  **no special privileges** — `claim` is permissionless — so this key only needs to hold gas.

## Setup

From `usc-messaging/` (the top-level package, not `usc-messaging/contracts/`):

```bash
npm install
cp .env.example .env
```

Fill in the "Bridge claimer bot" section of `.env`:

| Variable                   | Description                                                                                                                                                                                                                                                          |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `CREDITCOIN_RPC_URL`       | Shared with the rest of this package — the Creditcoin RPC `BridgeHub` lives on.                                                                                                                                                                                      |
| `CLAIMER_ROUTES`           | JSON array, one entry per spoke chain to watch: `[{"chainKey":8,"rpcUrl":"https://...","bridgeVaultAddress":"0x...","confirmationDepth":2}]`. `chainKey` is the registry chain key (**not** the EVM chain id) — the same one `BridgeVault`/`BridgeHub`/`Outbox` use. |
| `BRIDGE_HUB_ADDRESS`       | `BridgeHub` contract address on Creditcoin.                                                                                                                                                                                                                          |
| `PROOF_GEN_URL`            | `proof-gen-api-server` base URL (the same service `asc-message-relayer`'s ack path uses).                                                                                                                                                                            |
| `CLAIM_SIGNER_KEY`         | Gas-funded Creditcoin private key that submits `BridgeHub.claim`.                                                                                                                                                                                                    |
| `CLAIMER_POLL_INTERVAL_MS` | How often each route re-polls for new `Deposited` logs. Default `5000`.                                                                                                                                                                                              |

A route's `confirmationDepth` is how many blocks to hold back from that spoke's tip before treating
a `Deposited` log as final enough to start proving — a reorg past that depth isn't something this
bot recovers from on its own (see "Known limitations").

## Running as a daemon

```bash
npm run bridge-claimer-bot
```

This runs `tsx src/bridge-claimer-bot/index.ts`, which watches every route in `CLAIMER_ROUTES`
forever, claiming deposits as they're seen and attested. `Ctrl-C` stops all watchers cleanly.

On every start (including a restart), each route rewinds a fixed 600-block lookback from its
current tip rather than resuming an exact cursor — see "Known limitations" for why this is safe
but not maximally efficient.

## One-shot manual claim

To prove and submit a single specific deposit without running the daemon (useful for a manual dry
run, a demo, or nudging a deposit that's stuck):

```bash
tsx src/bridge-claimer-bot/claim.ts <sourceChainKey> <depositTxHash>
```

This looks up the matching route in `CLAIMER_ROUTES` by `sourceChainKey`, fetches the deposit
transaction's receipt from that chain to find its block number, and then runs the exact same
`claimDeposit()` path the daemon uses per-event.

## Protection against double-submission

Three independent layers, from cheapest/weakest to authoritative:

1. **`ClaimTracker`** (`pending.ts`) — in-memory, per-process dedup so the same deposit isn't
   proven/submitted twice concurrently, and a failed claim backs off instead of retrying in a
   tight loop.
2. **Pre-flight `BridgeHub.claimed(key)` view call** — avoids a wasted proof fetch if a racing
   submitter (a user, or another bot instance) already claimed this deposit.
3. **`BridgeHub`'s on-chain `claimed` mapping** — the real backstop. `claim` is CEI-safe and
   idempotent, so even if both layers above are bypassed (e.g. a fresh process with no tracker
   state), a duplicate submission just reverts cheaply instead of double-releasing funds.

The claim key must match `BridgeHub`'s exactly — both `claim.ts` and `index.ts` compute it as
`keccak256(abi.encode(uint32 sourceChainKey, address vault, uint256 nonce))`.

## Error handling and retries

`claim.ts` classifies every failure to submit a claim:

- **Terminal** (a decoded on-chain revert that will never succeed on retry —
  `AllDepositsAlreadyClaimed`, `WrongEmitter`, `ChainNotConfigured`): the deposit is dropped for
  good and logged.
- **Everything else** (RPC hiccups, proof-gen not ready yet, a nonce race, or any other transient
  condition): backed off exponentially (starting at 5s, doubling, capped at 5 minutes) and retried
  indefinitely — never a permanent give-up on its own.

## Known limitations (POC scope)

- **No durable checkpoint store.** A restart forgets in-flight/backoff state and each route
  rewinds a fixed 600-block lookback from the current tip rather than resuming an exact cursor.
  This is safe — `BridgeHub.claimed` plus the pre-flight check make re-observing an
  already-claimed deposit a no-op — but not efficient at scale.
- **In-memory-only dedup/backoff state.** Lost on every restart; only ever an optimization, never
  the correctness guarantee (that's `BridgeHub.claimed`, see above).
- **A reorg past `confirmationDepth` isn't handled.** It would need a manual re-scan; this bot
  doesn't detect or recover from it on its own.
- **Gas funding for `CLAIM_SIGNER_KEY` and destination-vault liquidity are operational concerns**,
  not something this bot manages — and destination-vault liquidity matters more than it used to.
  Release delivery goes through each spoke's shared `DispatcherRouter`
  (see `BridgeVault.sol`'s NatSpec), and a revert there — most plausibly `InsufficientLiquidity` on
  an under-funded vault — is reported back as a terminal, completed delivery, not a retryable
  pending one: there is no on-chain path to retry it once that happens. Keep destination vaults
  well-funded ahead of expected claim volume rather than relying on a top-up-and-retry recovery.
