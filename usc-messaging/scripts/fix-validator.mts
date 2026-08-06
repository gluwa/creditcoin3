// Fix: the reused June-era EOAValidator's validateVotes returns void; the post-#23 Inbox expects
// bool — decoding empty returndata reverts on every delivery. Deploy the current registry-based
// validator stack on Sepolia, seeded with the live attestor set, and swap it into the Inbox.
// Env: SEPOLIA_RPC, DEPLOYER_KEY (= Inbox owner).
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const UC = process.env.USC_CONTRACTS_DIR ?? "/Users/dylan/Projects/usc-contracts";
const ART = (p: string, n: string) =>
  JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? "/tmp/usc-dev-deploy.json";

const a = JSON.parse(readFileSync(OUT, "utf8"));
const OLD_VALIDATOR = "0x71A21Ea8d28D3a0618d61d478Ee20DCB64be8082";

const provider = new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC!, 11155111, { staticNetwork: true });
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY!, provider);

// Live attestor set from the old validator (just synced to 10 by the set-update pipeline).
const old = new ethers.Contract(OLD_VALIDATOR, ["function attestors() view returns (address[])"], provider);
const attestors: string[] = [...(await old.attestors())];
console.log("seeding registry with", attestors.length, "attestors from the old validator");

async function deploy(name: string, art: any, args: any[] = []) {
  const c = await new ethers.ContractFactory(art.abi, art.bytecode, wallet).deploy(...args);
  await c.waitForDeployment();
  console.log(`  ${name} → ${await c.getAddress()}`);
  return c;
}

const registry = await deploy("AttestorRegistry", ART("write-ability/AttestorRegistry.sol", "AttestorRegistry"),
  [wallet.address, attestors]);
// minAttestorCount=3, threshold = floor(20N/30)+1 = floor(2N/3)+1 — same formula as before.
const validator = await deploy("EOAValidator", ART("write-ability/EOAValidator.sol", "EOAValidator"),
  [wallet.address, await registry.getAddress(), 3, 20, 1]);
await (await (registry as any).setUpdater(await validator.getAddress(), true)).wait();
console.log("  registry.setUpdater(validator) done");

const inbox = new ethers.Contract(a.dest.inbox,
  ["function setDefaultVoteValidator(address)", "function defaultVoteValidator() view returns (address)"], wallet);
await (await inbox.setDefaultVoteValidator(await validator.getAddress())).wait();
console.log("  Inbox.defaultVoteValidator →", await inbox.defaultVoteValidator());

const v = new ethers.Contract(await validator.getAddress(),
  ["function attestors() view returns (address[])", "function threshold() view returns (uint256)"], provider);
console.log("  new validator: attestors", (await v.attestors()).length, "threshold", (await v.threshold()).toString());

a.dest.voteValidator = await validator.getAddress();
a.dest.attestorRegistry = await registry.getAddress();
a.dest.oldVoteValidator = OLD_VALIDATOR;
writeFileSync(OUT, JSON.stringify(a, null, 2));
console.log("✅ validator stack replaced");
process.exit(0);
