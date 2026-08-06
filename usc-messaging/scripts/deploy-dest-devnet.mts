// usc-dev destination-stack deploy (real Sepolia): SimpleInbox + MockDestination, REUSING the
// live EOAValidator (attestor _3 + relayer 0.1.1 already speak it; set is synced 10/7).
// Env: SEPOLIA_RPC, DEPLOYER_KEY. Artifacts from $USC_CONTRACTS_DIR (post-#23 build).
import { ethers } from "ethers";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const UC = process.env.USC_CONTRACTS_DIR ?? "/Users/dylan/Projects/usc-contracts";
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = process.env.DEPLOY_OUT ?? "./usc-dev-deploy.json";
const CHAIN_KEY = 8;
const CREDITCOIN_CHAIN_ID = 42;
const SEPOLIA_CHAIN_ID = 11155111;
const VALIDATOR = "0x71A21Ea8d28D3a0618d61d478Ee20DCB64be8082"; // live EOAValidator — reused
const LOCAL_CHAIN_KEY = ethers.zeroPadValue(ethers.toBeHex(CHAIN_KEY), 32);

const rpc = process.env.SEPOLIA_RPC!;
const key = process.env.DEPLOYER_KEY!;
if (!rpc || !key) throw new Error("need SEPOLIA_RPC + DEPLOYER_KEY");

const provider = new ethers.JsonRpcProvider(rpc, SEPOLIA_CHAIN_ID, { staticNetwork: true });
const wallet = new ethers.Wallet(key, provider);

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}

console.log("deployer:", wallet.address, "balance:", ethers.formatEther(await provider.getBalance(wallet.address)), "ETH");
// Post-#23: SimpleInbox is gone; Inbox(chainKey, creditcoinChainId, validator, messageDispatcher, owner)
// where the dispatcher must be a deployed contract — so the consumer dApp goes first.
const dapp = await deploy("MockDestination", ART("mocks/TestMocks.sol", "MockDestination"));
const inbox = await deploy("Inbox", ART("write-ability/Inbox.sol", "Inbox"),
  [LOCAL_CHAIN_KEY, CREDITCOIN_CHAIN_ID, VALIDATOR, await dapp.getAddress(), wallet.address]);

const addrs = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")) : {};
addrs.dest = {
  chainId: SEPOLIA_CHAIN_ID, chainKey: CHAIN_KEY, creditcoinChainId: CREDITCOIN_CHAIN_ID,
  localChainKey: LOCAL_CHAIN_KEY, voteValidator: VALIDATOR,
  inbox: await inbox.getAddress(), dapp: await dapp.getAddress(), admin: wallet.address,
};
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log("✅ dest stack deployed →", OUT);
process.exit(0);
