// devnet: deploy the SHARED Creditcoin-side write-ability contracts for a network, once.
//
// Step 0 when bringing write-ability up on a fresh devnet (NETWORK=asc-devnet by default):
//   0. deploy-shared-devnet.mts        (this)  — Creditcoin: the contracts every chain key reuses
//   1. pallet-config-devnet.mjs         — sudo, per chain key: factory, write-ability config, discovery
//   2. deploy-dest-chain-devnet.mts     — destination chain: AttestorRegistry, EOAValidator, router, Inbox
//   3. deploy-source-chain-devnet.mts   — Creditcoin: per-chain Outbox / vaults / Lite / ack validator
//
// Deploys (DEPLOYER_KEY = owner of everything below), recording each under `source.*`:
//   MockERC20("Attest","ATTEST")                   attest           (devnet stand-in for the ATTEST token)
//   ASCProofVerifier                               proofVerifier
//   EVMDeliveryDecoder(owner)                      deliveryDecoder  (bakes in Inbox.deliverMessage's selector)
//   AttestorRegistry(owner, [])                    attestorRegistry (vault-side registry; empty on devnet)
//   OutboxFactory                                  factory
//   FeeRegistry(chain-info precompile 0x…0FD3)     feeRegistry      (core fee stays in the runtime)
//   ChainRegistry(owner)                           chainRegistry
//   OutboxDiscovery impl + ERC1967Proxy(initialize(owner, chainRegistry))   outboxDiscovery(+Impl)
//
// Idempotent and resumable: a recorded address whose code is still on chain is reused, the record
// is written after every deployment, and the discovery proxy's owner / chainRegistry are verified.
// Nothing here is chain-key specific; registerOutbox / setChain happen in deploy-source-chain-devnet.
//
// Env:
//   NETWORK            asc-devnet (default) | usc-devnet            (network.mjs)
//   DEPLOYER_KEY       Creditcoin deployer, needs CTC for gas       (required)
//   ASC_CONTRACTS_DIR  compiled asc-contracts checkout (main 3f9259e+, `npx hardhat compile`)
//   CC_RPC / DEPLOY_OUT  override the NETWORK defaults
//
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { network, deployPath, ccProvider } from "./network.mjs";

const UC = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
if (!UC) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main 3f9259e+, `npx hardhat compile`)");
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const ARTX = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/${p}/${n}.json`, "utf8"));

const NET = network();
const OUT = deployPath(NET);
const need = (k: string): string => { const v = process.env[k]; if (!v) throw new Error(`missing ${k}`); return v; };
const DEPLOYER_KEY = need("DEPLOYER_KEY");
const CORE_FEE_PRECOMPILE = "0x0000000000000000000000000000000000000FD3";
const MIN_NATIVE = ethers.parseEther("1");

const deployJson = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")) : {};
deployJson.network ??= NET.name;
if (deployJson.network !== NET.name) throw new Error(`${OUT} belongs to network ${deployJson.network}, not ${NET.name}`);
deployJson.chains ??= {};
const s = (deployJson.source ??= {});
if (s.chainId !== undefined && Number(s.chainId) !== NET.evmChainId) {
  throw new Error(`${OUT} source.chainId is ${s.chainId}, but NETWORK=${NET.name} is ${NET.evmChainId}`);
}

const { provider, rpc } = await ccProvider(NET, s.rpc);
const wallet = new ethers.Wallet(DEPLOYER_KEY, provider);
const owner = wallet.address;
const balance = await provider.getBalance(owner);
console.log(`network ${NET.name} (chain id ${NET.evmChainId}) via ${rpc}`);
console.log(`deployer ${owner}  balance ${ethers.formatEther(balance)} CTC  nonce ${await wallet.getNonce()}`);
if (balance < MIN_NATIVE) throw new Error(`deployer holds ${ethers.formatEther(balance)} CTC; fund it (≥ 1 CTC) before deploying`);
if (s.deployer && s.deployer.toLowerCase() !== owner.toLowerCase()) {
  throw new Error(`${OUT} source.deployer is ${s.deployer}; DEPLOYER_KEY derives ${owner} — the shared contracts must share one owner`);
}

const save = () => writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n");
const hasCode = async (addr: string) => (await provider.getCode(addr)) !== "0x";

async function ensure(key: string, label: string, art: any, args: any[] = []): Promise<ethers.Contract> {
  if (s[key]) {
    if (!(await hasCode(s[key]))) throw new Error(`source.${key} = ${s[key]} is recorded but has no code on ${NET.name}`);
    console.log(`  ${label} (existing) ${s[key]}`);
    return new ethers.Contract(s[key], art.abi, wallet);
  }
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  const addr = await c.getAddress();
  s[key] = addr;
  save();
  console.log(`  ${label} → ${addr}`);
  return new ethers.Contract(addr, art.abi, wallet);
}

s.chainId = NET.evmChainId;
s.rpc = rpc.replace(/\?.*$/, "");
s.deployer = owner;
save();

console.log("shared contracts:");
await ensure("attest", "MockERC20 ATTEST", ART("mocks/MockERC20.sol", "MockERC20"), ["Attest", "ATTEST"]);
await ensure("proofVerifier", "ASCProofVerifier", ART("write-ability/common/ASCProofVerifier.sol", "ASCProofVerifier"));
const decoder = await ensure("deliveryDecoder", "EVMDeliveryDecoder", ART("write-ability/common/EVMDeliveryDecoder.sol", "EVMDeliveryDecoder"), [owner]);
await ensure("attestorRegistry", "AttestorRegistry (vault side)", ART("write-ability/AttestorRegistry.sol", "AttestorRegistry"), [owner, []]);
const factory = await ensure("factory", "OutboxFactory", ART("write-ability/deployer/OutboxFactory.sol", "OutboxFactory"));
await ensure("feeRegistry", "FeeRegistry (chain-info precompile)", ART("write-ability/FeeRegistry.sol", "FeeRegistry"), [CORE_FEE_PRECOMPILE]);
const chainRegistry = await ensure("chainRegistry", "ChainRegistry", ART("write-ability/deployer/ChainRegistry.sol", "ChainRegistry"), [owner]);

const discoveryArt = ART("write-ability/deployer/OutboxDiscovery.sol", "OutboxDiscovery");
let discovery: ethers.Contract;
if (s.outboxDiscovery) {
  if (!(await hasCode(s.outboxDiscovery))) throw new Error(`source.outboxDiscovery = ${s.outboxDiscovery} has no code on ${NET.name}`);
  discovery = new ethers.Contract(s.outboxDiscovery, discoveryArt.abi, wallet);
  console.log(`  OutboxDiscovery (existing proxy) ${s.outboxDiscovery}`);
} else {
  const impl = await ensure("outboxDiscoveryImpl", "OutboxDiscovery (impl)", discoveryArt);
  const init = new ethers.Interface(discoveryArt.abi).encodeFunctionData("initialize", [owner, await chainRegistry.getAddress()]);
  const proxy = await ensure("outboxDiscovery", "OutboxDiscovery (ERC1967Proxy)",
    ARTX("@openzeppelin/contracts/proxy/ERC1967/ERC1967Proxy.sol", "ERC1967Proxy"), [await impl.getAddress(), init]);
  discovery = new ethers.Contract(await proxy.getAddress(), discoveryArt.abi, wallet);
  s.outboxDiscoveryDeployedAt = new Date().toISOString();
  save();
}

// Read-back sanity on the pieces with owner / wiring state.
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
if (!same(await discovery.owner(), owner)) throw new Error(`OutboxDiscovery owner is ${await discovery.owner()}, not the deployer`);
if (!same(await discovery.chainRegistry(), await chainRegistry.getAddress())) throw new Error("OutboxDiscovery.chainRegistry() does not point at source.chainRegistry");
if (!same(await decoder.owner(), owner)) throw new Error(`EVMDeliveryDecoder owner is ${await decoder.owner()}, not the deployer`);
if (!same(await chainRegistry.owner(), owner)) throw new Error(`ChainRegistry owner is ${await chainRegistry.owner()}, not the deployer`);
console.log(`  OutboxFactory.version(): ${await (factory as any).version()}`);

// The decoder bakes in Inbox.deliverMessage's selector; record which one this build targets so a
// later Inbox ABI change is caught (see redeploy-decoder-devnet.mts).
const inboxIface = new ethers.Interface(ART("write-ability/Inbox.sol", "Inbox").abi);
s.deliveryDecoderSelector = inboxIface.getFunction("deliverMessage")!.selector;
s.sharedDeployedAt ??= new Date().toISOString();
save();

console.log(`✅ shared stack ready on ${NET.name}; recorded in ${OUT}`);
console.log(`   deliverMessage selector ${s.deliveryDecoderSelector}`);
console.log("next, per chain key K:");
console.log(`   CHAIN_KEY=K SUDO_URI=… node scripts/pallet-config-devnet.mjs`);
console.log(`   CHAIN_KEY=K DEST_CHAIN_ID=… DEST_RPC=… DEPLOYER_KEY=… npx tsx scripts/deploy-dest-chain-devnet.mts`);
console.log(`   CHAIN_KEY=K DEST_RPC=… DEST_ADMIN_KEY=… DEPLOYER_KEY=… npx tsx scripts/deploy-source-chain-devnet.mts`);
process.exit(0);
