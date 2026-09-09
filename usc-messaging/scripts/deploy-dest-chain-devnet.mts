// usc-dev: deploy the DESTINATION-chain write-ability stack for a newly registered chain key
// (e.g. Base Sepolia, chain id 84532) — AttestorRegistry, EOAValidator, MockDestination, Inbox.
//
// Step 2 of 3 when adding a destination chain to usc-devnet (Creditcoin EVM chain id 42):
//   1. register-chain-devnet.mjs        — pallet side, assigns CHAIN_KEY
//   2. deploy-dest-chain-devnet.mts     (this)
//   3. deploy-source-chain-devnet.mts   — Creditcoin: per-chain Outbox / vault / RelayerContractLite,
//                                         then allowlists that Outbox on the Inbox deployed here.
//
// On-chain calls (destination chain, DEPLOYER_KEY):
//   AttestorRegistry(owner, INITIAL_ATTESTORS)
//   EOAValidator(owner, registry, MIN_ATTESTOR_COUNT, THRESHOLD_NUMERATOR, THRESHOLD_ADDITION)
//   MockDestination()                                        // Inbox needs a dispatcher WITH code
//   Inbox(bytes32(CHAIN_KEY), 42, validator, mockDestination, owner, [] /* initialOutboxes */)
//   AttestorRegistry.setUpdater(validator, true)             // lets submitAttestorSetUpdate rotate the set
//
// The registry cannot start empty: EOAValidator's constructor reverts "below minimum" unless the
// registry already holds >= MIN_ATTESTOR_COUNT attestors, so INITIAL_ATTESTORS defaults to the
// three placeholder addresses the anvil e2e uses. The owner must replace them with the real
// attestor EVM addresses (AttestorRegistry.updateAttestorSet) before any delivery is attempted.
//
// Records chains.<CHAIN_KEY>.dest in the deploy JSON (rpc stored WITHOUT path/query, i.e. no API key).
//
// Env:
//   CHAIN_KEY             chain key assigned by register-chain-devnet.mjs   (required)
//   DEST_RPC              destination-chain JSON-RPC URL                     (required)
//   DEST_CHAIN_ID         destination EVM chain id, e.g. 84532               (required)
//   DEPLOYER_KEY          destination-chain deployer, needs native gas       (required)
//   INITIAL_ATTESTORS     comma-separated EVM addresses (default: 3 placeholders, see above)
//   MIN_ATTESTOR_COUNT / THRESHOLD_NUMERATOR / THRESHOLD_ADDITION   default 3 / 20 / 1
//   ASC_CONTRACTS_DIR     compiled asc-contracts checkout (main c83b3372+)
//   DEPLOY_OUT            default ../usc-dev-deploy.json
//
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

function ascContractsDir(): string {
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+, `npx hardhat compile`)");
  return dir;
}
const UC = ascContractsDir();
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;

const need = (k: string): string => {
  const v = process.env[k];
  if (!v) throw new Error(`missing ${k}`);
  return v;
};
const uint = (k: string, dflt?: number): number => {
  const v = process.env[k] ?? (dflt !== undefined ? String(dflt) : undefined);
  if (v === undefined) throw new Error(`missing ${k}`);
  if (!/^\d+$/.test(v)) throw new Error(`${k} must be a non-negative integer, got "${v}"`);
  return Number(v);
};

const CHAIN_KEY = uint("CHAIN_KEY");
const DEST_CHAIN_ID = uint("DEST_CHAIN_ID");
const DEST_RPC = need("DEST_RPC");
const DEPLOYER_KEY = need("DEPLOYER_KEY");
const MIN_ATTESTOR_COUNT = uint("MIN_ATTESTOR_COUNT", 3);
const THRESHOLD_NUMERATOR = uint("THRESHOLD_NUMERATOR", 20);
const THRESHOLD_ADDITION = uint("THRESHOLD_ADDITION", 1);
const CREDITCOIN_CHAIN_ID = 42;
const MIN_NATIVE = ethers.parseEther("0.02");
if (CHAIN_KEY === 0) throw new Error("CHAIN_KEY must be > 0 (Inbox rejects bytes32(0))");

// Same placeholders as deploy-dest-ethers.mts; wiped by the owner via updateAttestorSet later.
const INITIAL_ATTESTORS = (process.env.INITIAL_ATTESTORS
  ? process.env.INITIAL_ATTESTORS.split(",").map((a) => a.trim()).filter(Boolean)
  : ["0x0000000000000000000000000000000000000001", "0x0000000000000000000000000000000000000002", "0x0000000000000000000000000000000000000003"]
).map((a) => ethers.getAddress(a));
if (new Set(INITIAL_ATTESTORS.map((a) => a.toLowerCase())).size !== INITIAL_ATTESTORS.length) throw new Error("INITIAL_ATTESTORS has duplicates");
if (INITIAL_ATTESTORS.length < MIN_ATTESTOR_COUNT) {
  throw new Error(`INITIAL_ATTESTORS has ${INITIAL_ATTESTORS.length} entries; EOAValidator needs >= MIN_ATTESTOR_COUNT (${MIN_ATTESTOR_COUNT}) in the registry at construction`);
}

// localChainKey = chain_key_to_bytes32(CHAIN_KEY): value in the low 8 bytes (matches the Rust encoder).
const LOCAL_CHAIN_KEY = ethers.zeroPadValue(ethers.toBeHex(CHAIN_KEY), 32);
// Persist the RPC without path/query so an embedded provider API key never lands in git.
const rpcForRecord = new URL(DEST_RPC).origin;

const deployJson = JSON.parse(readFileSync(OUT, "utf8"));
deployJson.chains ??= {};
const entry = deployJson.chains[String(CHAIN_KEY)];
if (!entry) throw new Error(`no chains.${CHAIN_KEY} in ${OUT} — run register-chain-devnet.mjs first`);
if (entry.destChainId !== undefined && Number(entry.destChainId) !== DEST_CHAIN_ID) {
  throw new Error(`chains.${CHAIN_KEY}.destChainId is ${entry.destChainId}, but DEST_CHAIN_ID=${DEST_CHAIN_ID}`);
}
if (entry.dest?.inbox) {
  throw new Error(`chains.${CHAIN_KEY}.dest.inbox is already ${entry.dest.inbox}; refusing to redeploy (remove the entry to force)`);
}

const provider = new ethers.JsonRpcProvider(DEST_RPC, DEST_CHAIN_ID, { staticNetwork: true });
const wallet = new ethers.Wallet(DEPLOYER_KEY, provider);
const owner = wallet.address;

// staticNetwork skips the handshake, so confirm we are really on DEST_CHAIN_ID before spending gas.
const liveChainId = Number(BigInt(await provider.send("eth_chainId", [])));
if (liveChainId !== DEST_CHAIN_ID) throw new Error(`${rpcForRecord} reports chain id ${liveChainId}, expected DEST_CHAIN_ID=${DEST_CHAIN_ID}`);

const balance: bigint = await provider.getBalance(owner);
console.log(`deployer ${owner}  balance ${ethers.formatEther(balance)} native  nonce ${await wallet.getNonce()}  chain ${DEST_CHAIN_ID} (${rpcForRecord})`);
if (balance < MIN_NATIVE) throw new Error(`deployer has ${ethers.formatEther(balance)} native, needs at least ${ethers.formatEther(MIN_NATIVE)}`);
console.log(`chain key ${CHAIN_KEY} → localChainKey ${LOCAL_CHAIN_KEY}; initial attestors: ${INITIAL_ATTESTORS.join(", ")}`);

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}

const registry = await deploy("AttestorRegistry", ART("write-ability/AttestorRegistry.sol", "AttestorRegistry"), [owner, INITIAL_ATTESTORS]);
const registryAddr = await registry.getAddress();
// 2/3 + 1 quorum (numerator 20 / THRESHOLD_DENOMINATOR 30, addition 1), minAttestorCount at the floor.
const validator = await deploy("EOAValidator", ART("write-ability/EOAValidator.sol", "EOAValidator"),
  [owner, registryAddr, MIN_ATTESTOR_COUNT, THRESHOLD_NUMERATOR, THRESHOLD_ADDITION]);
const validatorAddr = await validator.getAddress();
// Inbox requires messageDispatcher to already have code, so the dApp goes first.
const dapp = await deploy("MockDestination", ART("mocks/TestMocks.sol", "MockDestination"));
const dappAddr = await dapp.getAddress();
// asc-contracts #48: six-arg constructor; initialOutboxes stays empty — the Outbox only exists after
// deploy-source-chain-devnet, which allowlists it via setSupportedOutbox.
const inbox = await deploy("Inbox", ART("write-ability/Inbox.sol", "Inbox"),
  [LOCAL_CHAIN_KEY, CREDITCOIN_CHAIN_ID, validatorAddr, dappAddr, owner, []]);
const inboxAddr = await inbox.getAddress();

// Same as asc-contracts scripts/hardhat/deployWriteAbility.ts: the validator's
// submitAttestorSetUpdate writes through to the registry, so it must be an authorised updater.
const registryC = registry as unknown as ethers.Contract;
if (!(await registryC.isUpdater(validatorAddr))) {
  await (await registryC.setUpdater(validatorAddr, true)).wait();
  console.log("  AttestorRegistry.setUpdater(EOAValidator, true)");
} else console.log("  AttestorRegistry already has EOAValidator as updater");

// Read-back sanity.
const inboxC = inbox as unknown as ethers.Contract;
if ((await inboxC.localChainKey()).toLowerCase() !== LOCAL_CHAIN_KEY.toLowerCase()) throw new Error("Inbox.localChainKey() mismatch");
if (Number(await inboxC.creditcoinChainId()) !== CREDITCOIN_CHAIN_ID) throw new Error("Inbox.creditcoinChainId() != 42");
if ((await inboxC.defaultVoteValidator()).toLowerCase() !== validatorAddr.toLowerCase()) throw new Error("Inbox.defaultVoteValidator() mismatch");

deployJson.chains[String(CHAIN_KEY)] = {
  ...entry,
  dest: {
    chainId: DEST_CHAIN_ID, rpc: rpcForRecord, chainKey: CHAIN_KEY,
    creditcoinChainId: CREDITCOIN_CHAIN_ID, localChainKey: LOCAL_CHAIN_KEY,
    voteValidator: validatorAddr, attestorRegistry: registryAddr,
    inbox: inboxAddr, dapp: dappAddr, admin: owner,
    initialAttestors: INITIAL_ATTESTORS, deployedAt: new Date().toISOString(),
  },
};
writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n");
console.log(`✅ dest stack for chain key ${CHAIN_KEY} deployed → chains.${CHAIN_KEY}.dest in ${OUT}`);
console.log(`   Inbox ${inboxAddr} has NO supported Outbox yet; deploy-source-chain-devnet.mts allowlists it.`);
console.log(`   Replace the placeholder attestors: AttestorRegistry(${registryAddr}).updateAttestorSet([...]) as ${owner}.`);
process.exit(0);
