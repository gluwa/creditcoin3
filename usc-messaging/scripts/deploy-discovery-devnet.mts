// usc-dev: deploy the Outbox discovery registry on CREDITCOIN (chain 42) and register the live
// Outbox as the default for chain key 8. This is prerequisite (b) of the discovery cutover
// (creditcoin3 #1292 / asc-message-relayer #50 resolve the Outbox only through this registry):
//
//   ChainRegistry(owner) ── setChain(8, 11155111)
//   OutboxDiscovery impl ── ERC1967Proxy(impl, initialize(owner, chainRegistry))
//   OutboxDiscovery.registerOutbox(8, <source.outbox>)   // first registration = default, no timelock
//
// It does NOT call `supportedChains.set_outbox_discovery_addr` (prerequisite (c)): that
// extrinsic exists only once the #1292 runtime is live on usc-dev. Run pallet-side after that.
//
// Idempotent: reuses source.chainRegistry / source.outboxDiscovery from usc-dev-deploy.json when
// present and only fixes up the registration. Owner of both contracts = the deployer wallet.
//
// Env: DEPLOYER_KEY (usc-dev deployer, devnet-only), ASC_CONTRACTS_DIR (compiled asc-contracts at
// main c83b3372 or later), optional CC_RPC, DEPLOY_OUT (defaults to ../usc-dev-deploy.json).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

function ascContractsDir(): string {
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+, `npx hardhat compile`)");
  return dir;
}
const UC = ascContractsDir();
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const CHAIN_KEY = 8;
const DEST_CHAIN_ID = 11155111; // Sepolia
const CC_CHAIN_ID = 42;
if (!process.env.DEPLOYER_KEY) throw new Error("missing DEPLOYER_KEY");

const rpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
const provider = new ethers.JsonRpcProvider(rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY, provider);

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const s = addrs.source;
if (!s?.outbox) throw new Error("usc-dev-deploy.json has no source.outbox");
if (Number(s.chainKey ?? CHAIN_KEY) !== CHAIN_KEY) throw new Error(`source.chainKey is ${s.chainKey}, expected ${CHAIN_KEY}`);
console.log(`deployer ${wallet.address}  balance ${ethers.formatEther(await provider.getBalance(wallet.address))} CTC`);
console.log(`Outbox to register: ${s.outbox} (chain key ${CHAIN_KEY} → Sepolia ${DEST_CHAIN_ID})`);

// Sanity: the Outbox must answer chainKey() == 8 or registerOutbox reverts InvalidOutbox.
const outboxKey = Number(await new ethers.Contract(s.outbox, ["function chainKey() view returns (uint32)"], provider).chainKey());
if (outboxKey !== CHAIN_KEY) throw new Error(`Outbox.chainKey() is ${outboxKey}, expected ${CHAIN_KEY}`);

// 1. ChainRegistry — maps chain key → destination EVM chain id; registerOutbox requires a mapping.
const registryArt = ART("contracts/write-ability/deployer/ChainRegistry.sol", "ChainRegistry");
let registry: ethers.Contract;
if (s.chainRegistry) {
  registry = new ethers.Contract(s.chainRegistry, registryArt.abi, wallet);
  console.log(`  ChainRegistry (existing) ${s.chainRegistry}`);
} else {
  registry = (await deploy("ChainRegistry", registryArt, [wallet.address])) as any;
}
if (Number(await registry.chainIdOf(CHAIN_KEY)) !== DEST_CHAIN_ID) {
  await (await registry.setChain(CHAIN_KEY, DEST_CHAIN_ID)).wait();
  console.log(`  ChainRegistry.setChain(${CHAIN_KEY}, ${DEST_CHAIN_ID})`);
}

// 2. OutboxDiscovery behind a plain ERC-1967 proxy (UUPS, initialize(owner, registry)).
const discoveryArt = ART("contracts/write-ability/deployer/OutboxDiscovery.sol", "OutboxDiscovery");
let discovery: ethers.Contract;
let implAddr = s.outboxDiscoveryImpl ?? null;
if (s.outboxDiscovery) {
  discovery = new ethers.Contract(s.outboxDiscovery, discoveryArt.abi, wallet);
  console.log(`  OutboxDiscovery (existing proxy) ${s.outboxDiscovery}`);
} else {
  const impl = await deploy("OutboxDiscovery (impl)", discoveryArt);
  implAddr = await impl.getAddress();
  const init = new ethers.Interface(discoveryArt.abi).encodeFunctionData("initialize", [wallet.address, await registry.getAddress()]);
  const proxy = await deploy("OutboxDiscovery (ERC1967Proxy)",
    ART("@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol", "ERC1967Proxy"), [implAddr, init]);
  discovery = new ethers.Contract(await proxy.getAddress(), discoveryArt.abi, wallet);
}
const owner = await discovery.owner();
if (owner.toLowerCase() !== wallet.address.toLowerCase()) throw new Error(`OutboxDiscovery owner is ${owner}, not the deployer`);
if ((await discovery.chainRegistry()).toLowerCase() !== (await registry.getAddress()).toLowerCase()) {
  throw new Error("OutboxDiscovery.chainRegistry() does not point at our ChainRegistry");
}

// 3. Register the live Outbox. First registration for the key becomes the default immediately.
const current: string = await discovery.defaultOutbox(CHAIN_KEY);
if (current.toLowerCase() === s.outbox.toLowerCase()) {
  console.log(`  defaultOutbox(${CHAIN_KEY}) already ${current}`);
} else {
  if (current !== ethers.ZeroAddress) {
    throw new Error(`defaultOutbox(${CHAIN_KEY}) is already ${current}; a rotation needs setDefaultOutbox + timelock, not this script`);
  }
  await (await discovery.registerOutbox(CHAIN_KEY, s.outbox)).wait();
  const after: string = await discovery.defaultOutbox(CHAIN_KEY);
  if (after.toLowerCase() !== s.outbox.toLowerCase()) throw new Error(`registerOutbox landed but defaultOutbox is ${after}`);
  console.log(`  registerOutbox(${CHAIN_KEY}, ${s.outbox}) → defaultOutbox confirmed`);
}

addrs.source = {
  ...s,
  chainRegistry: await registry.getAddress(),
  outboxDiscovery: await discovery.getAddress(),
  outboxDiscoveryImpl: implAddr,
  outboxDiscoveryDeployedAt: s.outboxDiscoveryDeployedAt ?? new Date().toISOString(),
};
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log(`✅ discovery ready on Creditcoin. Next (after the #1292 runtime is live): sudo supportedChains.set_outbox_discovery_addr(${CHAIN_KEY}, ${await discovery.getAddress()})`);
process.exit(0);
