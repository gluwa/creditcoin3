// Authorize an additional quoter EOA on the usc-dev USCRelayingQuoter, so a partner (e.g. Kevin's
// bridge) can sign fee quotes with their OWN key instead of sharing the deployer/master key.
// Env: DEPLOYER_KEY (quoter owner), QUOTER_EOA (address to authorize), optional CC_RPC / DEPLOY_OUT.
// Usage: QUOTER_EOA=0x... DEPLOYER_KEY=0x... npx tsx scripts/add-quoter.mts
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const UC = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR ?? "/Users/dylan/Projects/asc-contracts";
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = process.env.DEPLOY_OUT ?? "usc-dev-deploy.json";
const CC_CHAIN_ID = 42;

const rpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
const key = process.env.DEPLOYER_KEY!;
if (!key) throw new Error("need DEPLOYER_KEY (the quoter contract owner)");
const quoterEOA = process.env.QUOTER_EOA!;
if (!ethers.isAddress(quoterEOA)) throw new Error("need QUOTER_EOA (address to authorize)");

const provider = new ethers.JsonRpcProvider(rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(key, provider);

async function main() {
  const addrs = JSON.parse(readFileSync(OUT, "utf8"));
  const quoterAddr = addrs.source?.quoter;
  if (!quoterAddr) throw new Error(`no source.quoter in ${OUT}`);

  const art = ART("write-ability/USCRelayingQuoter.sol", "USCRelayingQuoter");
  const quoter = new ethers.Contract(quoterAddr, art.abi, wallet);

  const already = await quoter.isAuthorizedQuoter(quoterEOA).catch(() => null);
  if (already === true) {
    console.log(`${quoterEOA} is already an authorized quoter on ${quoterAddr} — nothing to do`);
    return;
  }

  console.log(`addQuoter(${quoterEOA}) on ${quoterAddr} as ${wallet.address} ...`);
  const tx = await quoter.addQuoter(quoterEOA);
  const rcpt = await tx.wait();
  console.log(`  done in tx ${rcpt!.hash} (block ${rcpt!.blockNumber})`);

  const check = await quoter.isAuthorizedQuoter(quoterEOA).catch(() => null);
  console.log(`  isAuthorizedQuoter(${quoterEOA}) = ${check}`);
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
