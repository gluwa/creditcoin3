# USC Audit Automation

A Deno-based TypeScript tool that runs attestation sanity checks on USC
(Creditcoin3) and reports to Slack or stdout.

All configuration is loaded from a single JSON file. For CI, env overrides:
`USC_NOTI_SLACK_BOT_TOKEN`, `USC_NOTI_SLACK_CHANNEL_ID`,
`USC_SLACK_ALERT_GROUP`, `SEPOLIA_RPC_URL`, `BSC_MAINNET_RPC_URL`,
`MAINNET_RPC_URL`

## Features

- Validates attestation block height vs the source chain's current block, using
  each chain's maturity strategy read from USC storage (see below)
- Verifies attestation header hash matches Ethereum block
- Checks checkpoint creation is within expected range
- Compares on-chain data with GraphQL indexer
- Sends formatted reports to Slack (or stdout with `--no-slack`)

## Requirements

- [Deno](https://deno.land/) 2.x

This project uses `deno.lock` for dependency pinning. The root `yarn.lock` is
for Node.js packages elsewhere in the repo—both should be committed.

## Quick Start

```bash
# Local report only (no Slack)
deno task start -- --config config-devnet.json --no-slack

# With Slack (add slackBotToken and slackChannelId to config)
deno task start -- --config config-devnet.json
```

## Configuration

Create a JSON config file. All settings live here—no `.env` or environment
variables.

```json
{
  "uscWsUrl": "wss://rpc.cc3-devnet.creditcoin.network",
  "uscNetworkName": "Creditcoin3 Devnet",
  "graphqlUrl": "https://graphql-usc.cc3-devnet.creditcoin.network",
  "ethRpc": [
    {
      "chainId": 11155111,
      "chainKey": 2,
      "url": "wss://ethereum-sepolia.publicnode.com"
    },
    { "chainId": 56, "chainKey": 8, "url": "https://bsc-rpc.publicnode.com" }
  ],
  "slackBotToken": "xxxx-xxxxxxxxxx-xx...",
  "slackChannelId": "C09DC0AAD..",
  "slackAlertGroup": "U123456"
}
```

- **uscWsUrl**, **graphqlUrl**: Required
- **ethRpc**: Array of `{ chainId, chainKey?, url }`; `chainKey` optional
  (discovered from USC if omitted)
- **balanceChecks**: Optional native/EVM account balance checks. Use
  `type: "substrate"` for SS58 accounts queried through `uscWsUrl`; omit `type`
  for EVM accounts queried through Blockscout/`eth_getBalance`.
- **slackBotToken**, **slackChannelId**, **slackAlertGroup**: Optional; required
  only when not using `--no-slack`

**Env overrides (CI)**: `SEPOLIA_RPC_URL` overrides url for chainId 11155111;
`BSC_MAINNET_RPC_URL` for chainId 56 and `MAINNET_RPC_URL` for chainId 1.

Devnet's chainId 56 entry ships a public BSC endpoint, so `BSC_MAINNET_RPC_URL`
is optional; set it to move that check onto a private provider.

Relative config paths (e.g. `config-devnet.json`) are resolved from the script
directory, so it works regardless of current working directory.

## Maturity strategy

The allowed lag between the source chain tip and the last attested block is
derived from the chain's on-chain maturity strategy (`SupportedChains` storage);
it is not configured.

- `EvmFinalized` (64), `EvmSafe` (32), `EvmLatest` (0) and `FixedDelay: N` are
  fixed offsets behind the tip.
- `RpcSafe` / `RpcFinalized` use the lag between the tip and the source node's
  `safe` / `finalized` block, measured on each run. If that query fails, the
  check falls back to `EvmSafe` with a warning.
- Each case adds 3 attestation intervals of slack.
- If the strategy is missing or unrecognised, the audit falls back to `EvmSafe`
  with a warning, and the report shows `EvmSafe (fallback)`.

## CLI

| Argument        | Description                         |
| --------------- | ----------------------------------- |
| `-c, --config`  | Path to JSON config file (required) |
| `--no-slack`    | Skip Slack; print to stdout only    |
| `-v, --verbose` | Verbose logging                     |

## Pre-configured Files

- `config-devnet.json` - Creditcoin3 Devnet
- `config-testnet.json` - Creditcoin USC Testnet
- `config-mainnet.json` - Creditcoin3 Mainnet

## Development

```bash
deno task dev -- --config config-devnet.json --no-slack
deno task fmt
deno task lint
deno task check
deno task test
```

## Cron / Scheduled Runs

```bash
*/15 * * * * cd /path/to/creditcoin3-next/scripts/usc-audit-automation && deno task start -- --config config-devnet.json
```
