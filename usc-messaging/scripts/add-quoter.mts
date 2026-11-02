// Authorize an additional quoter EOA on the usc-dev USCRelayingQuoter, so a partner (e.g. Kevin's
// bridge) can sign fee quotes with their OWN key instead of sharing the deployer/master key.
// Env: DEPLOYER_KEY (quoter owner), QUOTER_EOA (address to authorize), optional CC_RPC / DEPLOY_OUT.
// Usage: QUOTER_EOA=0x... DEPLOYER_KEY=0x... npx tsx scripts/add-quoter.mts
import { ethers } from "ethers";
import { readFileSync } from "node:fs";
import { network, deployPath, ccRpc, ccProvider } from "./network.mjs";

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

const NET = network();
const OUT = deployPath(NET);
const CC_CHAIN_ID = NET.evmChainId;

const rpc = ccRpc(NET);
const key = process.env.DEPLOYER_KEY!;
if (!key) throw new Error("need DEPLOYER_KEY (the quoter contract owner)");
const quoterEOA = process.env.QUOTER_EOA!;
if (!ethers.isAddress(quoterEOA)) throw new Error("need QUOTER_EOA (address to authorize)");

const { provider } = await ccProvider(NET, rpc);
const wallet = new ethers.Wallet(key, provider);

async function main() {
  const addrs = JSON.parse(readFileSync(OUT, "utf8"));
  const quoterAddr = addrs.source?.quoter;
  if (!quoterAddr) throw new Error(`no source.quoter in ${OUT}`);

  const art = ART("write-ability/ASCRelayingQuoter.sol", "ASCRelayingQuoter");
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
