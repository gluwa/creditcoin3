// Fix: the Outbox was deployed with the quoter in the feeRegistry slot (stale e2e arg order).
// Deploy the real FeeRegistry (backed by the chain-info precompile's get_core_fee) and swap it
// in via Outbox.setFeeRegistry (owner-only). Env: DEPLOYER_KEY (= Outbox owner).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const UC = process.env.USC_CONTRACTS_DIR ?? "/Users/dylan/Projects/usc-contracts";
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
