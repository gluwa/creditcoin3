// Plain-ethers destination-stack deploy (anvil): AttestorRegistry + EOAValidator + Inbox +
// MockDestination. Artifacts come from the usc-contracts hardhat build. Run with tsx (Node 22).
// Post-#23: SimpleInbox is gone — Inbox takes a fixed messageDispatcher (the dApp) set at
// construction, and EOAValidator delegates its attestor set to a shared AttestorRegistry.
import { ethers } from "ethers";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

// asc-contracts checkout: $ASC_CONTRACTS_DIR, else the sibling of this repo (…/Projects/asc-contracts).
function ascContractsDir(): string {
  // ASC_CONTRACTS_DIR since the repo was renamed usc-contracts -> asc-contracts; the old name is
  // still accepted so existing setups keep working. Deliberately no default: this used to fall
  // back to one developer's home directory, so anyone else got a confusing "cannot read artifact"
  // three calls later instead of being told what to set.
  const dir = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
  if (!dir) {
    throw new Error(
      "set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (run `npx hardhat compile` there first)",
    );
  }
  return dir;
}

const UC = ascContractsDir();
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = "/tmp/e2e-deploy.json";
const CHAIN_KEY = 2;
const CREDITCOIN_CHAIN_ID = 42;
// localChainKey = chain_key_to_bytes32(2): value in the low 8 bytes (matches the Rust encoder).
const LOCAL_CHAIN_KEY = ethers.zeroPadValue(ethers.toBeHex(CHAIN_KEY), 32);
const ANVIL0 = "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";

const provider = new ethers.JsonRpcProvider("http://127.0.0.1:8545", 31337, { staticNetwork: true });
provider.pollingInterval = 500;
const wallet = new ethers.Wallet(ANVIL0, provider);

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}

// Placeholder attestor set: launch-attestors.sh replaces this wholesale via
// AttestorRegistry.updateAttestorSet once the live attestors are known. MIN_ATTESTOR_COUNT_FLOOR
// (3) forces at least 3 distinct seed addresses even as a placeholder — any non-zero, non-duplicate
// addresses work since they're wiped before delivery is exercised.
const PLACEHOLDER_ATTESTORS = [
  "0x0000000000000000000000000000000000000001",
  "0x0000000000000000000000000000000000000002",
  "0x0000000000000000000000000000000000000003",
];
const registry = await deploy("AttestorRegistry", ART("write-ability/AttestorRegistry.sol", "AttestorRegistry"),
  [wallet.address, PLACEHOLDER_ATTESTORS]);
// 2/3 + 1 quorum (numerator 20 / THRESHOLD_DENOMINATOR 30, addition 1), minAttestorCount at the floor.
const validator = await deploy("EOAValidator", ART("write-ability/EOAValidator.sol", "EOAValidator"),
  [wallet.address, await registry.getAddress(), 3, 20, 1]);
const dapp = await deploy("MockDestination", ART("mocks/TestMocks.sol", "MockDestination"));
// Inbox requires its messageDispatcher to already have code, so the dApp must be deployed first.
const inbox = await deploy("Inbox", ART("write-ability/Inbox.sol", "Inbox"),
  [LOCAL_CHAIN_KEY, CREDITCOIN_CHAIN_ID, await validator.getAddress(), await dapp.getAddress(), wallet.address]);

const addrs = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")) : {};
addrs.dest = {
  chainId: 31337, rpc: "http://127.0.0.1:8545", chainKey: CHAIN_KEY,
  creditcoinChainId: CREDITCOIN_CHAIN_ID, localChainKey: LOCAL_CHAIN_KEY,
  voteValidator: await validator.getAddress(), attestorRegistry: await registry.getAddress(),
  inbox: await inbox.getAddress(), dapp: await dapp.getAddress(), admin: wallet.address,
};
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log("✅ dest stack deployed →", OUT);
process.exit(0);
