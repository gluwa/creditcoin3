#!/usr/bin/env tsx

import "dotenv/config";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { requireEnv, getPayeeAddress, getDestinationAddress, runCommand } from "../src/utils";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const REPO_ROOT = path.resolve(__dirname, "..");
const CONTRACTS_DIR = path.join(REPO_ROOT, "contracts");
const ENV_FILE = path.join(REPO_ROOT, ".env");

function parseDeployedAddress(output: string, label: string): string {
  const match = output.match(/Deployed to:\s*(0x[a-fA-F0-9]{40})/);
  if (!match) {
    throw new Error(`Failed to parse deployed address for ${label}\n${output}`);
  }
  return match[1];
}

function deployToDestination(contractSpec: string, constructorArgs: string[] = []): string {
  const rpcUrl = requireEnv("DESTINATION_CHAIN_RPC_URL");
  const privateKey = requireEnv("DESTINATION_CHAIN_PRIVATE_KEY");

  const args = [
    "create",
    "--rpc-url",
    rpcUrl,
    "--private-key",
    privateKey,
    "--broadcast",
    contractSpec,
    ...(
      constructorArgs.length > 0
        ? ["--constructor-args", ...constructorArgs]
        : []
    ),
  ];

  const output = runCommand("forge", args, CONTRACTS_DIR);
  process.stderr.write(output);
  return parseDeployedAddress(output, contractSpec);
}

function deployToSource(contractSpec: string, constructorArgs: string[] = []): string {
  const rpcUrl = requireEnv("CREDITCOIN_RPC_URL");
  const privateKey = requireEnv("CREDITCOIN_CHAIN_PRIVATE_KEY");

  const args = [
    "create",
    "--rpc-url",
    rpcUrl,
    "--private-key",
    privateKey,
    "--broadcast",
    contractSpec,
    ...(
      constructorArgs.length > 0
        ? ["--constructor-args", ...constructorArgs]
        : []
    ),
  ];

  const output = runCommand("forge", args, CONTRACTS_DIR);
  process.stderr.write(output);
  return parseDeployedAddress(output, contractSpec);
}

function updateEnvVar(key: string, value: string): void {
  const newLine = `${key}="${value}"`;

  let text = existsSync(ENV_FILE) ? readFileSync(ENV_FILE, "utf8") : "";
  const pattern = new RegExp(`^${key}=.*$`, "m");

  if (pattern.test(text)) {
    text = text.replace(pattern, newLine);
  } else {
    if (text && !text.endsWith("\n")) {
      text += "\n";
    }
    text += `${newLine}\n`;
  }

  writeFileSync(ENV_FILE, text, "utf8");
}

function getDestinationChainId(): string {
  const rpcUrl = requireEnv("DESTINATION_CHAIN_RPC_URL");
  const output = runCommand(
    "cast",
    ["chain-id", "--rpc-url", rpcUrl],
    CONTRACTS_DIR,
  );
  return output.trim();
}

async function main(): Promise<void> {
  const sourceChainId = process.env.SOURCE_CHAIN_ID ?? "42";
  const localChainKey =
    process.env.LOCAL_CHAIN_KEY ??
    "0x0000000000000000000000000000000000000000000000000000000000000001";

  const creditcoinRpcUrl = requireEnv("CREDITCOIN_RPC_URL");
  const destinationRpcUrl = requireEnv("DESTINATION_CHAIN_RPC_URL");

  const payee = getPayeeAddress(CONTRACTS_DIR);
  const destinationPublicKey = getDestinationAddress(CONTRACTS_DIR);

  console.log(
    `Deploying to source: ${creditcoinRpcUrl}, destination: ${destinationRpcUrl}...`,
  );

  // Source chain
  const relayer = deployToSource(
    "src/SimpleRelayer.sol:SimpleRelayer",
    [payee],
  );
  const outbox = deployToSource("src/SimpleOutbox.sol:Outbox");
  const dapp = deployToSource("src/SimpleDApp.sol:SimpleDApp", [outbox]);

  // Destination chain
  const validator = deployToDestination(
    "src/DummyVoteValidator.sol:DummyVoteValidator",
  );
  const inbox = deployToDestination(
    "src/SimpleInbox.sol:SimpleInbox",
    [validator, sourceChainId, localChainKey],
  );
  const destination = deployToDestination(
    "src/TestDestination.sol:TestDestination",
    [inbox]
  );
  const destinationChainId = getDestinationChainId();
  console.log(`Destination chainId: ${destinationChainId}`);

  // Write addresses back into .env
  updateEnvVar("INBOX_ADDR", inbox);
  updateEnvVar("VOTE_VALIDATOR_ADDR", validator);
  updateEnvVar("DESTINATION_CONTRACT_ADDR", destination);
  updateEnvVar("OUTBOX_ADDR", outbox);
  updateEnvVar("RELAYER_CONTRACT_ADDR", relayer);
  updateEnvVar("DAPP_CONTRACT_ADDR", dapp);
  updateEnvVar("DESTINATION_CHAIN_ID", destinationChainId);
  updateEnvVar("DESTINATION_CHAIN_PUBLIC_KEY", destinationPublicKey);

  console.log(`DummyVoteValidator: ${validator}`);
  console.log(`SimpleInbox: ${inbox}`);
  console.log(`SimpleOutbox: ${outbox}`);
  console.log(`SimpleDApp: ${dapp}`);
  console.log(`TestDestination: ${destination}`);
  console.log(`RelayerContract: ${relayer}`);
  console.log(`Updated ${ENV_FILE}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
