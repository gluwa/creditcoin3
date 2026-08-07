// Hand the usc-dev price-oracle role to a partner EOA (or back to ours). The role is a SINGLE
// address on both contracts (TWAPReader.update + USCRelayingQuoter.priceUpdate are `onlyOracle`),
// so whoever holds it is the only account that can keep quotes fresh — with it transferred, our
// own publish-devnet.mts TWAP refresh reverts UnauthorizedOracle until the role is set back
// (ownership stays with DEPLOYER_KEY, so switching back is always one run of this script).
// Env: DEPLOYER_KEY (contract owner), ORACLE_EOA (new oracle), optional CC_RPC / DEPLOY_OUT.
// Usage: ORACLE_EOA=0x... DEPLOYER_KEY=0x... npx tsx scripts/set-oracle-devnet.mts
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const UC = process.env.USC_CONTRACTS_DIR ?? "/Users/dylan/Projects/usc-contracts";
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));

const OUT = process.env.DEPLOY_OUT ?? "usc-dev-deploy.json";
const CC_CHAIN_ID = 42;

const rpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
const key = process.env.DEPLOYER_KEY!;
if (!key) throw new Error("need DEPLOYER_KEY (the contracts' owner)");
const oracle = process.env.ORACLE_EOA!;
if (!ethers.isAddress(oracle)) throw new Error("need ORACLE_EOA (address to hold the oracle role)");

const provider = new ethers.JsonRpcProvider(rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(key, provider);

async function main() {
  const addrs = JSON.parse(readFileSync(OUT, "utf8"));
  const targets: Array<[string, string, string, string]> = [
    ["TWAPReader", addrs.source?.twapReader, "write-ability/TWAPReader.sol", "TWAPReader"],
    ["USCRelayingQuoter", addrs.source?.quoter, "write-ability/USCRelayingQuoter.sol", "USCRelayingQuoter"],
  ];
  for (const [label, addr, artPath, artName] of targets) {
    if (!addr) throw new Error(`missing address for ${label} in ${OUT}`);
    const c = new ethers.Contract(addr, ART(artPath, artName).abi, wallet);
    const current = await c.oracleService();
    if (current.toLowerCase() === oracle.toLowerCase()) {
      console.log(`${label} ${addr}: oracle already ${oracle} — nothing to do`);
      continue;
    }
    console.log(`${label} ${addr}: setOracleService(${oracle}) (was ${current}) ...`);
    const rcpt = await (await c.setOracleService(oracle)).wait();
    const after = await c.oracleService();
    console.log(`  done in tx ${rcpt!.hash}; oracleService now ${after}`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
