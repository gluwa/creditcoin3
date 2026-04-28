/**
 * Configuration for USC Audit Automation
 *
 * Loads from a single JSON config file. No environment variables.
 */

import { parseArgs } from "@std/cli/parse-args";
import type { BalanceNetworkConfig } from "./balances.ts";

export interface EthRpcConfig {
  chainId: number;
  chainKey?: number;
  chainName?: string;
  url: string;
}

export interface AuditConfig {
  uscWsUrl: string;
  uscNetworkName: string;
  graphqlUrl: string;
  ethRpc: EthRpcConfig[];
  slackBotToken?: string;
  slackChannelId?: string;
  slackAlertGroup?: string;
  noSlack: boolean;
  verbose: boolean;
  balanceChecks?: BalanceNetworkConfig[];
}

/**
 * Resolve config path: relative paths are resolved relative to the project directory (parent of src/).
 */
function resolveConfigPath(configPath: string): string {
  if (configPath.startsWith("/") || /^[A-Za-z]:/.test(configPath)) {
    return configPath;
  }
  const projectDir = new URL("..", import.meta.url);
  const url = new URL(configPath, projectDir);
  return url.pathname;
}

function loadConfigFile(path: string): Record<string, unknown> {
  const content = Deno.readTextFileSync(path);
  return JSON.parse(content) as Record<string, unknown>;
}

export function loadConfig(): AuditConfig {
  const rawArgs = Deno.args[0] === "--" ? Deno.args.slice(1) : Deno.args;
  const args = parseArgs(rawArgs, {
    string: ["config"],
    boolean: ["no-slack", "verbose", "help"],
    default: {},
    alias: { c: "config", h: "help", v: "verbose" },
  });

  if (args.help) {
    printHelp();
    Deno.exit(0);
  }

  const configPathArg = typeof args.config === "string" ? args.config : null;
  if (!configPathArg) {
    console.error("Error: --config <path> is required");
    printHelp();
    Deno.exit(1);
  }

  const configPath = resolveConfigPath(configPathArg);
  let obj: Record<string, unknown>;
  try {
    obj = loadConfigFile(configPath);
  } catch (e) {
    throw new Error(`Failed to load config from ${configPath}: ${e}`);
  }

  const noSlack = args["no-slack"] === true;
  const verbose = args.verbose === true;

  const uscWsUrl = obj.uscWsUrl as string;
  const graphqlUrl = obj.graphqlUrl as string;
  const uscNetworkName = (obj.uscNetworkName as string) ?? "USC";

  if (!uscWsUrl || !graphqlUrl) {
    throw new Error(
      "uscWsUrl and graphqlUrl are required in config file",
    );
  }

  const ethRpcRaw = obj.ethRpc as
    | Array<
      { chainId: number; chainKey?: number; chainName?: string; url: string }
    >
    | undefined;
  const sepoliaUrl = Deno.env.get("SEPOLIA_RPC_URL");
  const bscUrl = Deno.env.get("BSC_RPC_URL");
  const mainnetEthUrl = Deno.env.get("MAINNET_RPC_URL");
  const ethRpc = (ethRpcRaw ?? []).map((r) => {
    let url = r.url;
    if (r.chainId === 11155111 && sepoliaUrl) url = sepoliaUrl;
    if (r.chainId === 97 && bscUrl) url = bscUrl;
    if (r.chainId === 1 && mainnetEthUrl) url = mainnetEthUrl;
    return {
      chainId: r.chainId,
      chainKey: r.chainKey,
      chainName: r.chainName,
      url,
    };
  });

  let slackBotToken: string | undefined;
  let slackChannelId: string | undefined;
  let slackAlertGroup: string | undefined;
  if (!noSlack) {
    slackBotToken = Deno.env.get("USC_NOTI_SLACK_BOT_TOKEN") ??
      (obj.slackBotToken as string) ?? undefined;
    slackChannelId = Deno.env.get("USC_NOTI_SLACK_CHANNEL_ID") ??
      (obj.slackChannelId as string) ?? undefined;
    slackAlertGroup = Deno.env.get("USC_SLACK_ALERT_GROUP") ??
      (obj.slackAlertGroup as string) ?? undefined;
    if (!slackBotToken) {
      throw new Error(
        "slackBotToken required in config (or use --no-slack for local report)",
      );
    }
    if (!slackChannelId) {
      throw new Error(
        "slackChannelId required in config (or use --no-slack for local report)",
      );
    }
  }
  const balanceChecks = obj.balanceChecks as BalanceNetworkConfig[] | undefined;

  return {
    uscWsUrl,
    uscNetworkName,
    graphqlUrl,
    ethRpc,
    slackBotToken,
    slackChannelId,
    slackAlertGroup,
    noSlack,
    verbose,
    balanceChecks,
  };
}

function printHelp(): void {
  console.log(`
USC Audit Automation

Runs attestation sanity checks on USC and reports to Slack or stdout.
All configuration is loaded from a single JSON file.

USAGE:
  deno task start -- --config <path> [options]

OPTIONS:
  -c, --config <path>   Path to JSON config file (required)
  --no-slack            Skip Slack; print report to stdout only
  -v, --verbose         Verbose logging

CONFIG FILE FORMAT (JSON):
  {
    "uscWsUrl": "wss://rpc.cc3-devnet.creditcoin.network",
    "uscNetworkName": "Creditcoin3 Devnet",
    "graphqlUrl": "https://attestations-graphql.cc3-devnet.creditcoin.network",
    "ethRpc": [
      { "chainId": 11155111, "chainKey": 2, "url": "wss://ethereum-sepolia.publicnode.com" },
      { "chainId": 97, "chainKey": 3, "url": "wss://bsc-testnet.publicnode.com" }
    ],
    "slackBotToken": "xxxx-xxxxxxxxxx-xx..."
    "slackChannelId": "C09DC0AAD...",
    "slackAlertGroup": "U123456",
    "balanceChecks": [
      {
        "name": "Creditcoin3 Devnet",
        "baseUrl": "https://creditcoin-devnet.blockscout.com/",
        "rpcUrl": "https://rpc.cc3-devnet.creditcoin.network",
        "accounts": [
          {
            "address": "0x053e6100c1C9A31e5c8a56156649f0972dCC815e",
            "name": "Devnet bigbag"
          },
          {
            "address": "0x465ac26CD80E6e6AADe0C2EdfF2518D03Ac4E4e5",
            "name": "Devnet traffic sim"
          },
          {
            "address": "0x2C317383b8A99537c61042B63C4fB042Df51033a",
            "name": "NEW_LENDER_ accnt in 1Password. Used for testing"
          }
        ]
      }
    ]
  }

  chainKey: optional; if omitted, discovered from USC storage by chainId
  slackBotToken, slackChannelId, slackAlertGroup: optional; required only when not using --no-slack

ENV OVERRIDES (for CI):
  SEPOLIA_RPC_URL   Override ethRpc url for chainId 11155111
  BSC_RPC_URL       Override ethRpc url for chainId 97
  MAINNET_RPC_URL   Override ethRpc url for chainId 1
`);
}
