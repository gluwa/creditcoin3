# USC Audit Automation

A Deno-based TypeScript tool that runs attestation sanity checks on USC
(Creditcoin3) and reports to Slack or stdout.

All configuration is loaded from a single JSON file. For CI, env overrides:
`USC_NOTI_SLACK_BOT_TOKEN`, `USC_NOTI_SLACK_CHANNEL_ID`,
`USC_SLACK_ALERT_GROUP`, `SEPOLIA_RPC_URL`, `BSC_RPC_URL`, `MAINNET_RPC_URL`

## Features

- Validates attestation block height vs Ethereum current block
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
    { "chainId": 97, "chainKey": 3, "url": "wss://bsc-testnet.publicnode.com" }
  ],
  "slackBotToken": "xxxx-xxxxxxxxxx-xx...",
  "slackChannelId": "C09DC0AAD..",
  "slackAlertGroup": "U123456"
}
```

- **uscWsUrl**, **graphqlUrl**: Required
- **ethRpc**: Array of `{ chainId, chainKey?, url }`; `chainKey` optional
  (discovered from USC if omitted)
- **slackBotToken**, **slackChannelId**, **slackAlertGroup**: Optional; required
  only when not using `--no-slack`

**Env overrides (CI)**: `SEPOLIA_RPC_URL` overrides url for chainId 11155111;
`BSC_RPC_URL` for chainId 97 and `MAINNET_RPC_URL` for chainId 1.

Relative config paths (e.g. `config-devnet.json`) are resolved from the script
directory, so it works regardless of current working directory.

## CLI

| Argument        | Description                         |
| --------------- | ----------------------------------- |
| `-c, --config`  | Path to JSON config file (required) |
| `--no-slack`    | Skip Slack; print to stdout only    |
| `-v, --verbose` | Verbose logging                     |

## Pre-configured Files

- `config-devnet.json` - Creditcoin3 Devnet
- `config-testnet.json` - Creditcoin USC Testnet
- `config-testnet-v1.json` - Creditcoin USC Testnet V1 (legacy, old release)

## Development

```bash
deno task dev -- --config config-devnet.json --no-slack
deno task fmt
deno task lint
deno task check
```

## Cron / Scheduled Runs

```bash
*/15 * * * * cd /path/to/creditcoin3-next/scripts/usc-audit-automation && deno task start -- --config config-devnet.json
```
