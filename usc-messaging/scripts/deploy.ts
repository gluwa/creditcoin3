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

function getDestinationDeployerAddress(): string {
  const privateKey = requireEnv("DESTINATION_CHAIN_PRIVATE_KEY");
  const output = runCommand(
    "cast",
    ["wallet", "address", "--private-key", privateKey],
    CONTRACTS_DIR,
  );
  return output.trim();
}

// Initial attestor EVM set the EOAValidator is seeded with at deploy time. The authoritative set is
// only known once the attestors launch (step 3), so we seed best-effort here and `launch-attestors.sh`
// syncs the live discovered set via `updateAttestorSet` afterwards. Prefer a `.attestor-set` written
// by a previous launch; otherwise fall back to the config.yaml defaults so the constructor has a
// valid (non-empty) set.
const ATTESTOR_SET_FILE = path.join(__dirname, ".attestor-set");
const DEFAULT_ATTESTORS = [
  "0x3C3224ECf3e12ec671D200a2802a2525Fa1B04aC",
  "0x0aC32750Ed79f301248afD9B398cc5723911c392",
  "0x4910156288781F080d81c607E3a830a7019d9Bc6",
];

function readInitialAttestors(): string[] {
  if (existsSync(ATTESTOR_SET_FILE)) {
    const set = readFileSync(ATTESTOR_SET_FILE, "utf8")
      .trim()
      .split(",")
      .map((a) => a.trim())
      .filter(Boolean);
    if (set.length > 0) {
      return set;
    }
  }
  return DEFAULT_ATTESTORS;
}

async function main(): Promise<void> {
  // Creditcoin L1 EVM chain id (eth_chainId). On `--dev` this is SS58Prefix = 42.
  const sourceChainId = process.env.SOURCE_CHAIN_ID ?? "42";
  // chain_key of the destination chain, given as a plain number in the env. In the dev genesis,
  // chain_key 2 = the local anvil ("Anvil1", chain id 31337) — the same chain_key the attestor
  // zombienet attests; 3 = Sepolia. `chainKeyBytes32` is the left-padded bytes32 form contracts use.
  const chainKeyU64 = BigInt(process.env.DESTINATION_CHAIN_KEY ?? "2");
  const chainKeyBytes32 = `0x${chainKeyU64.toString(16).padStart(64, "0")}`;

  const creditcoinRpcUrl = requireEnv("CREDITCOIN_RPC_URL");
  const destinationRpcUrl = requireEnv("DESTINATION_CHAIN_RPC_URL");

  const payee = getPayeeAddress();

  console.log(
    `Deploying to source: ${creditcoinRpcUrl}, destination: ${destinationRpcUrl}...`,
  );

  // Outbox is created via the factory, not deployed directly: deploy the USC-operated factory
  // first, then have it create the outbox for our chain key (the factory passes its own owner to
  // the outbox). This mirrors the "create factory first -> use factory to create outbox" pattern.
  const outboxFactory = deployToSource(
    "src/SimpleOutboxFactory.sol:OutboxFactory",
  );
  // chainKeyU64 (above) is also the destination chain key the AcknowledgmentValidator proves
  // MessageDelivered events on.

  // Trust-minimized acknowledgment validator (research §05/§10): verifies a native USC delivery
  // proof (block-prover precompile: merkle inclusion + continuity) that MessageDelivered was emitted
  // in a finalized block on the destination chain, decodes the messageIds, and acknowledges them on
  // the Outbox. It is the Outbox's `validator` (acknowledgeMessage is onlyValidator). Created first
  // (without the Outbox), then `setOutbox` once the factory has created the Outbox.
  const ackValidator = deployToSource(
    "src/AcknowledgmentValidator.sol:AcknowledgmentValidator",
    [chainKeyU64.toString(), payee], // (destinationChainKey, owner)
  );
  castSendSource(outboxFactory, "createOutbox(bytes32,address)", [
    chainKeyBytes32,
    ackValidator, // the Outbox's onlyValidator ack authority
  ]);
  const outbox = castCallSource(outboxFactory, "getOutbox(bytes32)(address)", [
    chainKeyBytes32,
  ]);
  castSendSource(ackValidator, "setOutbox(address)", [outbox]);

  // Register the factory on-chain so the attestor/relayer can resolve the outbox via the chain-info
  // precompile.
  console.log(
    `Registering OutboxFactory ${outboxFactory} for chain_key ${chainKeyU64} with pallet-supported-chains...`,
  );
  await registerOutboxFactory(chainKeyU64, outboxFactory);

  const dapp = deployToSource("src/SimpleDApp.sol:SimpleDApp", [outbox]);

  // Destination chain
  // Production vote validator: EOAValidator verifies attestor ECDSA signatures + 2/3+1 threshold on
  // every deliverMessage (replaces the always-accept DummyVoteValidator). Seeded with a best-effort
  // attestor set; launch-attestors.sh syncs the live discovered set via updateAttestorSet (the
  // destination deployer is the validator admin). threshold = 2/3 + 1, minAttestorCount = 1.
  const validatorAdmin = getDestinationDeployerAddress();
  const initialAttestors = readInitialAttestors();
  const validator = deployToDestination("src/EOAValidator.sol:EOAValidator", [
    validatorAdmin,
    `[${initialAttestors.join(",")}]`,
    "1", // minAttestorCount
    "2", // thresholdNumerator
    "3", // thresholdDenominator
    "1", // thresholdAddition
  ]);
  const destination = deployToDestination(
    "src/TestDestination.sol:TestDestination",
  );
  const inbox = deployToDestination(
    "src/SimpleInbox.sol:SimpleInbox",
    [validator, sourceChainId, chainKeyBytes32],
  );
  const destinationChainId = getDestinationChainId();
  console.log(`Destination chainId: ${destinationChainId}`);

  // Write addresses back into .env
  updateEnvVar("INBOX_ADDR", inbox);
  updateEnvVar("VOTE_VALIDATOR_ADDR", validator);
  updateEnvVar("DESTINATION_CONTRACT_ADDR", destination);
  updateEnvVar("OUTBOX_FACTORY_ADDR", outboxFactory);
  updateEnvVar("OUTBOX_ADDR", outbox);
  updateEnvVar("ACK_VALIDATOR_ADDR", ackValidator);
  updateEnvVar("DAPP_CONTRACT_ADDR", dapp);
  updateEnvVar("DESTINATION_CHAIN_ID", destinationChainId);

  console.log(`EOAValidator: ${validator} (admin ${validatorAdmin}, ${initialAttestors.length} seed attestors)`);
  console.log(`SimpleInbox: ${inbox}`);
  console.log(`OutboxFactory: ${outboxFactory}`);
  console.log(`SimpleOutbox (via factory): ${outbox}`);
  console.log(`AcknowledgmentValidator (outbox validator): ${ackValidator}`);
  console.log(`SimpleDApp: ${dapp}`);
  console.log(`TestDestination: ${destination}`);
  console.log(`Updated ${ENV_FILE}`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
