# Creditcoin Bridge (POC)

A Next.js dApp for bridging native testnet ETH between **Ethereum Sepolia** and **Base Sepolia**,
using Creditcoin as an invisible hub. You connect a wallet on one spoke chain, deposit into
`BridgeVault`, and receive the funds on the other spoke chain — Creditcoin's `BridgeHub` and a
standalone claimer bot handle everything in between; you never interact with Creditcoin directly.

See the write-ability bridge POC design for the full system (contracts, claimer bot, deploy
scripts) — this README only covers the dApp.

## How it works

1. **Deposit** — connect a wallet on a spoke chain, pick a destination, amount, and recipient
   (defaults to yourself), and submit. This locks funds in that chain's `BridgeVault`.
2. **Progress** — after a deposit, you land on a status page (`/progress/<depositTxHash>`) that
   tracks four steps: deposited → attested → claimed on Creditcoin → released on the destination.
   This page is reload-safe: the URL alone (just the deposit tx hash) is enough to rebuild the
   whole page from on-chain state, no wallet or local storage required.
3. **Claim** — claiming is fully automatic (a standalone bot submits the proof to `BridgeHub`).
   There is no manual "claim now" button — `/claim/<depositTxHash>` is a status-only view of the
   same progress.
4. **History** — `/history` lists transfers for your connected wallet (scanned directly from chain
   logs, no indexer) and lets you look up any transfer by its deposit tx hash.
5. **Inspect** — `/inspect` is a read-only debugging view of the deployed `BridgeHub` and each
   configured `BridgeVault`: owner, balances, per-chain config, ATTEST fee allowances, and small
   lookup tools for a claim key or a release messageId.

The one authoritative "done" signal is the `Released` event on the destination chain — everything
before that is progress information. Once a deposit is claimed on Creditcoin, delivery to the
destination is normally automatic and near-immediate via the existing message-relayer. The progress
screen also detects the one case (an under-funded destination vault) where delivery fails instead of
just being delayed — see [Notes and known limitations](#notes-and-known-limitations).

## Prerequisites

- Node.js 24 and Yarn (or npm)
- A [WalletConnect Cloud](https://cloud.walletconnect.com) project ID (free) — required by
  RainbowKit even if you only ever use an injected wallet like MetaMask
- Deployed `BridgeHub`/`BridgeVault` addresses for the chain(s) you want to test against — see
  [Deploying the contracts](#deploying-the-contracts) below if they don't exist yet — and a running
  `proof-gen-api-server` if you want the "attested" step to populate
- A running instance of the claimer bot (`usc-messaging/src/bridge-claimer-bot`) if you want
  deposits to actually reach "released" — without it, deposits stay stuck at "attested" until
  someone (the bot, or anyone) calls `BridgeHub.claim`

## Deploying the contracts

`BridgeHub` and `BridgeVault` aren't deployed by this dApp — they're deployed by scripts in the
parent repo's `usc-messaging/scripts/`. Run these from `usc-messaging/` (not `usc-bridge-dapp/`)
before filling in `.env.local` below.

Both contracts ride entirely on infra that already exists on each target devnet: the destination
release leg is delivered through each spoke chain's existing, shared `DispatcherRouter`/`Inbox`
and `Outbox`, and the already-running message-relayer — no dedicated `Inbox`/`Outbox`/vote
validator of our own to deploy or register. See `BridgeVault.sol`'s and `BridgeHub.sol`'s NatSpec
for the mechanism.

### 1. Build the contracts

```bash
cd usc-messaging/contracts
export ASC_CONTRACTS_DIR=/path/to/a/compiled/asc-contracts/checkout  # npx hardhat compile there first, pinned to the same commit as .github/workflows/bridge-contracts.yml
./scripts/setup-foundry-deps.sh
forge build
cd ..
```

### 2. Deploy `BridgeHub` (once, on Creditcoin)

```bash
export DEPLOYER_KEY=0x...
export CC_RPC=https://rpc.usc-devnet.creditcoin.network    # default shown; override for another devnet
export EXISTING_DEPLOY=./usc-dev-deploy.json                # must already exist (deploy-source-devnet.mts's output)
tsx scripts/deploy-bridge-hub.mts
```

Reuses that existing deploy's `ASCProofVerifier`, ATTEST token, `OutboxFactory`, `FeeRegistry`, and
`AttestorVault` rather than redeploying them, and writes the result to `usc-bridge-deploy.json`
(`DEPLOY_OUT` to change the path).

### 3. Deploy a `BridgeVault` spoke (once per spoke chain)

You'll need that spoke chain's already-live `DispatcherRouter` address (readable as that chain's
`Inbox.messageDispatcher()`) and its already-registered, canonical `Outbox` address on Creditcoin
(the one already allowlisted on that `Inbox` and already watched by the running message-relayer) —
ask whoever operates the target devnet for these if you don't have them on hand.

```bash
export SPOKE_RPC=https://sepolia.infura.io/v3/...
export SPOKE_CHAIN_ID=11155111       # 84532 for Base Sepolia
export DEPLOYER_KEY=0x...            # controls both the spoke and Creditcoin wallets
export SHARED_ROUTER=0x...           # that spoke's live DispatcherRouter address
export SHARED_OUTBOX=0x...           # that spoke's canonical Outbox address on Creditcoin
export CC_RPC=https://rpc.usc-devnet.creditcoin.network   # same devnet used in step 2
tsx scripts/deploy-bridge-spoke.mts <chainKey>             # e.g. 8 for Ethereum Sepolia
```

This deploys `BridgeVault` (trusting `SHARED_ROUTER` as its caller) and calls
`BridgeHub.setChainConfig(...)` to point that chain key at it and at `SHARED_OUTBOX` — nothing else
to configure; the existing message-relayer starts delivering to it immediately. Repeat with a
different `<chainKey>`/`SPOKE_*`/`SHARED_*` env vars for each additional spoke;
`usc-bridge-deploy.json` accumulates one `spokes.<chainKey>` entry per run. For a quick local dry
run you don't need two chains: deploying a single spoke once and using its chain key as both source
and destination lets it bridge deposits to itself.

### 4. Wire the addresses into the dApp

From the resulting `usc-messaging/usc-bridge-deploy.json`: copy `hub.bridgeHub` into
`NEXT_PUBLIC_BRIDGE_HUB_ADDRESS`, and each `spokes.<chainKey>.bridgeVault` into that chain's
`NEXT_PUBLIC_*_BRIDGE_VAULT_ADDRESS` (with `<chainKey>` itself as the matching
`NEXT_PUBLIC_*_CHAIN_KEY`) — see the env var table below.

## Setup

```bash
cd usc-bridge-dapp
yarn install
cp .env.example .env.local
```

Fill in `.env.local`:

| Variable                                                                                                | Required       | Description                                                                                                                                                                                                                                                            |
| ------------------------------------------------------------------------------------------------------- | -------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `NEXT_PUBLIC_WALLETCONNECT_PROJECT_ID`                                                                  | Recommended    | From cloud.walletconnect.com. The app builds without it, but the WalletConnect (QR code) option won't work — injected wallets still will.                                                                                                                              |
| `NEXT_PUBLIC_ETH_SEPOLIA_RPC_URL`                                                                       | No             | Defaults to a public Sepolia RPC.                                                                                                                                                                                                                                      |
| `NEXT_PUBLIC_BASE_SEPOLIA_RPC_URL`                                                                      | No             | Defaults to a public Base Sepolia RPC.                                                                                                                                                                                                                                 |
| `NEXT_PUBLIC_ETH_SEPOLIA_CHAIN_KEY` / `NEXT_PUBLIC_ETH_SEPOLIA_BRIDGE_VAULT_ADDRESS`                    | Yes, per spoke | The registry chain key (**not** the EVM chain id) and deployed `BridgeVault` address for Ethereum Sepolia.                                                                                                                                                             |
| `NEXT_PUBLIC_BASE_SEPOLIA_CHAIN_KEY` / `NEXT_PUBLIC_BASE_SEPOLIA_BRIDGE_VAULT_ADDRESS`                  | Yes, per spoke | Same, for Base Sepolia.                                                                                                                                                                                                                                                |
| `NEXT_PUBLIC_PROOF_GEN_URL`                                                                             | No             | `proof-gen-api-server` base URL. Without it, the "attested" step never completes (deposits will look stuck after step 1).                                                                                                                                              |
| `NEXT_PUBLIC_CREDITCOIN_CHAIN_ID` / `NEXT_PUBLIC_CREDITCOIN_RPC_URL` / `NEXT_PUBLIC_BRIDGE_HUB_ADDRESS` | No             | Optional, read-only. Only powers the best-effort "claimed" step; the dApp never asks for a Creditcoin wallet. Left unset, the progress page skips straight from "attested" to watching for the release.                                                                |
| `NEXT_PUBLIC_*_VAULT_GENESIS_BLOCK`                                                                     | No             | Block that spoke's `BridgeVault` was deployed at. Without it, History's chain-log scan starts from block 0 — correct but, on a chain with millions of blocks, so slow it can look like "no transfers ever show up." Find it once via a binary search on `eth_getCode`. |
| `NEXT_PUBLIC_*_ROUTER_ADDRESS`                                                                          | No             | That spoke's shared `DispatcherRouter` address (same as `SHARED_ROUTER` in step 3 above). Lets the progress screen and `/inspect` detect a terminal `DestinationDeliveryFailed` delivery; without it, a failed delivery just looks stuck at "released" forever.        |

A spoke only appears as a deposit source/destination once **both** its chain key and vault address
are set — set at least one spoke's pair, and a second one if you want to actually pick a
destination on the Deposit page.

### Regenerating the contract ABI

The ABI files under `src/lib/contracts/generated/` are committed but generated, not hand-written —
they come straight from the Foundry build so the dApp can never drift from what `BridgeVault`/
`BridgeHub` actually expose. After changing either contract:

```bash
# from usc-messaging/contracts/
forge build

# from usc-bridge-dapp/
yarn gen-abi
```

`yarn gen-abi --check` (what CI runs) exits non-zero instead of writing, if the committed files are
out of date.

## Running locally

```bash
yarn dev
```

Opens on <http://localhost:3000>. The root path redirects to `/deposit`.

To exercise a full round trip locally you'll also want, in separate terminals (from the repo's
`usc-messaging/` directory):

```bash
# one-shot: prove and submit a specific deposit tx yourself
tsx src/bridge-claimer-bot/claim.ts <sourceChainKey> <depositTxHash>

# or run the bot as a daemon that watches for and claims new deposits automatically
tsx src/bridge-claimer-bot/index.ts
```

Both read their configuration (`CLAIMER_ROUTES`, `BRIDGE_HUB_ADDRESS`, `PROOF_GEN_URL`,
`CLAIM_SIGNER_KEY`, ...) from `usc-messaging/.env` — see `usc-messaging/.env.example`.

## Scripts

| Command                             | Description                                                 |
| ----------------------------------- | ----------------------------------------------------------- |
| `yarn dev`                          | Start the dev server                                        |
| `yarn build`                        | Production build                                            |
| `yarn start`                        | Serve a production build                                    |
| `yarn lint`                         | ESLint                                                      |
| `yarn typecheck`                    | `tsc --noEmit`                                              |
| `yarn check-format` / `yarn format` | Prettier check / write                                      |
| `yarn gen-abi` [`--check`]          | Regenerate (or verify) the ABI files from the Foundry build |

## Notes and known limitations

- **Native ETH only.** `BridgeVault` also supports allow-listed ERC-20s, but the Deposit form only
  offers native ETH for now — no token has been allow-listed on any deployed vault yet.
- **No manual claim fallback.** Claiming always goes through the bot. This is deliberate: a manual
  fallback would need the user to hold a funded Creditcoin wallet and raw proof bytes, which
  defeats the point of an invisible hub. If claims are stuck, the fix is to check the bot, not to
  give users a harder way around it.
- **History is a live chain scan, not an indexer.** It scans both configured spokes' logs directly
  from the browser and caches only a scan cursor in `localStorage` — fine for one wallet's handful
  of transfers, not meant to scale further.
- **The "claimed" step is best-effort.** It disappears (shown as "not tracked") unless
  `NEXT_PUBLIC_BRIDGE_HUB_ADDRESS`/`NEXT_PUBLIC_CREDITCOIN_RPC_URL` are set. It never affects
  correctness — "released" is independently verified straight from the destination chain.
- **A release that fails at delivery time is not automatically retried.** Delivery goes through
  each spoke's shared `DispatcherRouter` with the release call carrying no native value, and for a
  zero-value call _any_ revert on `BridgeVault.releaseFromBridge` (most plausibly an under-funded
  destination vault) is reported back as a terminal, completed delivery rather than a pending one —
  there is no on-chain retry path for it, unlike the source-side claim leg's bot, which does retry.
  In practice this means: **keep destination vaults well-funded ahead of expected claim volume.**
  This is a known, accepted limitation for this POC (verified directly against the real
  `DispatcherRouter`/`Inbox` contracts), not a bug to work around client-side. The progress screen
  surfaces this as a distinct "Delivery failed" step (with the decoded revert reason, when
  recognized) instead of spinning forever — but only when that spoke's
  `NEXT_PUBLIC_*_ROUTER_ADDRESS` is set; without it, a failed delivery still just looks stuck.
