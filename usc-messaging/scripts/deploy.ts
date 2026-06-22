#!/usr/bin/env tsx

import "dotenv/config";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const REPO_ROOT = path.resolve(__dirname, "..");
const CONTRACTS_DIR = path.join(REPO_ROOT, "contracts");
const ENV_FILE = path.join(REPO_ROOT, ".env");

function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new Error(`Missing ${name}`);
  }
  return value;
}

function runCommand(cmd: string, args: string[], cwd: string): string {
  try {
    const output = execFileSync(cmd, args, {
      cwd,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return output;
  } catch (err: any) {
    const stdout = err?.stdout ? String(err.stdout) : "";
    const stderr = err?.stderr ? String(err.stderr) : "";
    const combined = [stdout, stderr].filter(Boolean).join("\n");
    throw new Error(`Command failed: ${cmd} ${args.join(" ")}\n${combined}`);
  }
}

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

function getPayeeAddress(): string {
  const privateKey = requireEnv("CREDITCOIN_CHAIN_PRIVATE_KEY");
  const output = runCommand(
    "cast",
    ["wallet", "address", "--private-key", privateKey],
    CONTRACTS_DIR,
  );
  return output.trim();
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

  const payee = getPayeeAddress();

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
  const destination = deployToDestination(
    "src/TestDestination.sol:TestDestination",
  );
  const inbox = deployToDestination(
    "src/SimpleInbox.sol:SimpleInbox",
    [validator, sourceChainId, localChainKey],
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
