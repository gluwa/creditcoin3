// Plain-ethers destination-stack deploy (anvil): EOAValidator + SimpleInbox + MockDestination.
// Artifacts come from the usc-contracts hardhat build. Run with tsx (Node 22).
import { ethers } from "ethers";
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";

// usc-contracts checkout: $USC_CONTRACTS_DIR, else the sibling of this repo (…/Projects/usc-contracts).
const UC = process.env.USC_CONTRACTS_DIR ?? fileURLToPath(new URL("../../../usc-contracts", import.meta.url));
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

// Placeholder attestor set (launch-attestors.sh syncs the live set; deployer is admin). 2/3+1.
const validator = await deploy("EOAValidator", ART("write-ability/EOAValidator.sol", "EOAValidator"),
  [wallet.address, [wallet.address], 1, 2, 3, 1]);
const inbox = await deploy("SimpleInbox", ART("write-ability/SimpleInbox.sol", "SimpleInbox"),
  [await validator.getAddress(), CREDITCOIN_CHAIN_ID, LOCAL_CHAIN_KEY]);
const dapp = await deploy("MockDestination", ART("mocks/TestMocks.sol", "MockDestination"));

const addrs = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")) : {};
addrs.dest = {
  chainId: 31337, rpc: "http://127.0.0.1:8545", chainKey: CHAIN_KEY,
  creditcoinChainId: CREDITCOIN_CHAIN_ID, localChainKey: LOCAL_CHAIN_KEY,
  voteValidator: await validator.getAddress(), inbox: await inbox.getAddress(),
  dapp: await dapp.getAddress(), admin: wallet.address,
};
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log("✅ dest stack deployed →", OUT);
process.exit(0);
