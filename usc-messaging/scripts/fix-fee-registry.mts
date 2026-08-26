// Remediation tool, no longer part of a fresh deploy. deploy-source-devnet.mts used to hand the
// quoter to the Outbox's feeRegistry slot, so coreFee() reverted and this had to be run after
// every deploy; that script now wires a real FeeRegistry itself. Kept because it is still the way
// to re-point an ALREADY-deployed Outbox: usc-devnet's live Outbox was created with the wrong slot
// and repaired by running this, and FeeRegistry is deliberately swappable (its own doc comment
// anticipates a storage-based registry replacing the precompile-backed one) via the owner-only
// Outbox.setFeeRegistry. Deploys a FeeRegistry backed by the chain-info precompile's get_core_fee
// and swaps it in. Env: DEPLOYER_KEY (= Outbox owner).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

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
const OUT = process.env.DEPLOY_OUT ?? "/tmp/usc-dev-deploy.json";
const CHAIN_INFO_PRECOMPILE = "0x0000000000000000000000000000000000000FD3";

const a = JSON.parse(readFileSync(OUT, "utf8"));
const s = a.source;
const provider = new ethers.JsonRpcProvider(s.rpc, s.chainId, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY!, provider);

const art = ART("write-ability/FeeRegistry.sol", "FeeRegistry");
const fr = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(CHAIN_INFO_PRECOMPILE);
await fr.waitForDeployment();
const frAddr = await fr.getAddress();
console.log("FeeRegistry →", frAddr, "(provider: chain-info precompile)");

const outbox = new ethers.Contract(s.outbox,
  ["function setFeeRegistry(address)", "function coreFee() view returns (uint256)", "function feeRegistry() view returns (address)"], wallet);
await (await outbox.setFeeRegistry(frAddr)).wait();
console.log("Outbox.feeRegistry →", await outbox.feeRegistry());
console.log("Outbox.coreFee() →", ethers.formatEther(await outbox.coreFee()), "ATTEST (pallet CoreFees(8), 0 until setCoreFee)");

s.feeRegistry = frAddr;
writeFileSync(OUT, JSON.stringify(a, null, 2));
console.log("✅ fee registry fixed");
process.exit(0);
