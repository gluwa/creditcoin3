#!/usr/bin/env tsx

import "dotenv/config";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ApiPromise, WsProvider, Keyring } from "@polkadot/api";
import { cryptoWaitReady } from "@polkadot/util-crypto";

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

function castSendSource(to: string, sig: string, args: string[]): void {
  const rpcUrl = requireEnv("CREDITCOIN_RPC_URL");
  const privateKey = requireEnv("CREDITCOIN_CHAIN_PRIVATE_KEY");
  const output = runCommand(
    "cast",
    ["send", to, sig, ...args, "--rpc-url", rpcUrl, "--private-key", privateKey],
    CONTRACTS_DIR,
  );
  process.stderr.write(output);
}

function castCallSource(to: string, sig: string, args: string[]): string {
  const rpcUrl = requireEnv("CREDITCOIN_RPC_URL");
  const output = runCommand(
    "cast",
    ["call", to, sig, ...args, "--rpc-url", rpcUrl],
    CONTRACTS_DIR,
  );
  return output.trim();
}

function toWs(url: string): string {
  return url.replace(/^http:\/\//, "ws://").replace(/^https:\/\//, "wss://");
}

// Register the deployed OutboxFactory with `pallet_supported_chains` so the attestor (and relayer)
// can resolve the outbox on-chain via the chain-info precompile (`outbox_factory_address` ->
// factory -> `getOutbox(chainKey)`). This is a Substrate extrinsic
// (`supportedChains.setOutboxFactoryAddr`), NOT an EVM call, and it is operator-gated — on a --dev
// node we submit it through sudo with the dev sudo account. The chain must already be supported.
//
// TODO(write-ability): for non-dev environments, submit this with a real Operators-membership key
//   instead of dev sudo, and inspect the `sudo.Sudid` event to surface inner-call failures (here we
//   only resolve once the wrapping extrinsic is in a block).
async function registerOutboxFactory(
  chainKey: bigint,
  factoryAddr: string,
): Promise<void> {
  // Note: `??` only falls back on undefined/null, but these env vars are commonly present-but-empty
  // (`CREDITCOIN_SUBSTRATE_WS_URL=""` in .env.example), so treat blank as unset.
  const configuredWs = process.env.CREDITCOIN_SUBSTRATE_WS_URL?.trim();
  const wsUrl = configuredWs ? configuredWs : toWs(requireEnv("CREDITCOIN_RPC_URL"));
  const sudoSuri = process.env.CREDITCOIN_SUDO_SURI?.trim() || "//Alice";

  const api = await ApiPromise.create({
    provider: new WsProvider(wsUrl),
    noInitWarn: true,
  });
  try {
    await api.isReady;
    await cryptoWaitReady();
    const sudo = new Keyring({ type: "sr25519" }).addFromUri(sudoSuri);

    await new Promise<void>((resolve, reject) => {
      api.tx.sudo
        .sudo(api.tx.supportedChains.setOutboxFactoryAddr(chainKey, factoryAddr))
        .signAndSend(sudo, ({ status, dispatchError }) => {
          if (dispatchError) {
            reject(
              new Error(`setOutboxFactoryAddr failed: ${dispatchError.toString()}`),
            );
          } else if (status.isInBlock || status.isFinalized) {
            resolve();
          }
        })
        .catch(reject);
    });
  } finally {
    await api.disconnect();
  }
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
  // Creditcoin L1 EVM chain id (eth_chainId). On `--dev` this is SS58Prefix = 42.
  const sourceChainId = process.env.SOURCE_CHAIN_ID ?? "42";
  // chain_key of the destination chain. In the dev genesis, chain_key 2 = the local anvil
  // ("Anvil1", chain id 31337) — the same chain_key the attestor zombienet attests.
  const localChainKey =
    process.env.LOCAL_CHAIN_KEY ??
    "0x0000000000000000000000000000000000000000000000000000000000000002";

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

  // Outbox is created via the factory, not deployed directly: deploy the USC-operated factory
  // first, then have it create the outbox for our chain key (the factory passes its own owner to
  // the outbox). This mirrors the "create factory first -> use factory to create outbox" pattern.
  const outboxFactory = deployToSource(
    "src/SimpleOutboxFactory.sol:OutboxFactory",
  );
  // TODO(write-ability): placeholder source-chain validator handed to the outbox. Once
  // acknowledgeMessage access control (delivery-proof verification) is implemented, replace this
  // with the real ack validator deployed on the source chain.
  const outboxValidator = deployToSource(
    "src/DummyVoteValidator.sol:DummyVoteValidator",
  );
  castSendSource(outboxFactory, "createOutbox(bytes32,address)", [
    localChainKey,
    outboxValidator,
  ]);
  const outbox = castCallSource(outboxFactory, "getOutbox(bytes32)(address)", [
    localChainKey,
  ]);

  // Register the factory on-chain so the attestor/relayer can resolve the outbox via the chain-info
  // precompile. The pallet's chain_key is the u64 in the low bytes of the bytes32 LOCAL_CHAIN_KEY.
  const chainKeyU64 = BigInt(localChainKey);
  console.log(
    `Registering OutboxFactory ${outboxFactory} for chain_key ${chainKeyU64} with pallet-supported-chains...`,
  );
  await registerOutboxFactory(chainKeyU64, outboxFactory);

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
  updateEnvVar("OUTBOX_FACTORY_ADDR", outboxFactory);
  updateEnvVar("OUTBOX_ADDR", outbox);
  updateEnvVar("RELAYER_CONTRACT_ADDR", relayer);
  updateEnvVar("DAPP_CONTRACT_ADDR", dapp);
  updateEnvVar("DESTINATION_CHAIN_ID", destinationChainId);

  console.log(`DummyVoteValidator: ${validator}`);
  console.log(`SimpleInbox: ${inbox}`);
  console.log(`OutboxFactory: ${outboxFactory}`);
  console.log(`SimpleOutbox (via factory): ${outbox}`);
  console.log(`SimpleDApp: ${dapp}`);
  console.log(`TestDestination: ${destination}`);
  console.log(`RelayerContract: ${relayer}`);
  console.log(`Updated ${ENV_FILE}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
