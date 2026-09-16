// Deploys BridgeHub once on Creditcoin. Reuses the existing devnet's ASCProofVerifier (stateless,
// chain-key-agnostic — "single ASC entry point ... used by bridge inbound, loan readability, and
// other ASC-compatible contracts", per its own NatSpec) and ATTEST token rather than redeploying
// them: BridgeHub is just another ASC-compatible consumer of infra that already exists on-chain.
//
// Per-spoke wiring (BridgeVault + dedicated Inbox on the spoke, dedicated Outbox on Creditcoin,
// BridgeHub.setChainConfig) is deploy-bridge-spoke.mts's job, run once per spoke chain key after
// this script.
//
// Env: CC_RPC (default devnet), DEPLOYER_KEY, EXISTING_DEPLOY (default ./usc-dev-deploy.json,
// read-only source for proofVerifier/attest/factory/feeRegistry/attestorVault), DEPLOY_OUT
// (default ./usc-bridge-deploy.json).
import { ethers } from "ethers";
import { readFileSync, writeFileSync, existsSync } from "node:fs";

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
// BridgeHub itself is built with Foundry, not the asc-contracts hardhat toolchain — its artifact
// shape differs (bytecode nested under .bytecode.object rather than a flat .bytecode string).
const ART_FOUNDRY = (contract: string) => {
  const j = JSON.parse(
    readFileSync(`${import.meta.dirname}/../contracts/out/${contract}.sol/${contract}.json`, "utf8"),
  );
  return { abi: j.abi, bytecode: j.bytecode.object as string };
};

const EXISTING = process.env.EXISTING_DEPLOY ?? "./usc-dev-deploy.json";
const OUT = process.env.DEPLOY_OUT ?? "./usc-bridge-deploy.json";
const CC_CHAIN_ID = 42;

const rpc = process.env.CC_RPC ?? "https://rpc.usc-devnet.creditcoin.network";
const key = process.env.DEPLOYER_KEY!;
if (!key) throw new Error("need DEPLOYER_KEY");

const provider = new ethers.JsonRpcProvider(rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
const wallet = new ethers.Wallet(key, provider);

async function deploy(name: string, art: { abi: any; bytecode: string }, args: any[] = []) {
  const f = new ethers.ContractFactory(art.abi, art.bytecode, wallet);
  const c = await f.deploy(...args);
  const addr = await c.getAddress();
  process.stdout.write(`  ${name} → ${addr}\n`);
  await c.waitForDeployment();
  return c;
}

async function main() {
  if (!existsSync(EXISTING)) {
    throw new Error(
      `${EXISTING} not found — run deploy-source-devnet.mts (or set EXISTING_DEPLOY) first, ` +
        "BridgeHub reuses that stack's ASCProofVerifier/ATTEST/OutboxFactory/FeeRegistry/AttestorVault",
    );
  }
  const existing = JSON.parse(readFileSync(EXISTING, "utf8"));
  const src = existing.source;
  for (const field of ["proofVerifier", "attest", "factory", "feeRegistry", "attestorVault"]) {
    if (!src?.[field]) throw new Error(`${EXISTING}: source.${field} missing`);
  }

  const owner = wallet.address;
  console.log(
    "deployer:", owner,
    "balance:", ethers.formatEther(await provider.getBalance(owner)),
    "CTC, nonce:", await wallet.getNonce(),
  );

  const hub = await deploy("BridgeHub", ART_FOUNDRY("BridgeHub"), [src.proofVerifier, src.attest, owner]);
  const hubAddr = await hub.getAddress();

  const out = existsSync(OUT) ? JSON.parse(readFileSync(OUT, "utf8")) : {};
  out.hub = {
    chainId: CC_CHAIN_ID,
    rpc,
    owner,
    proofVerifier: src.proofVerifier,
    attestToken: src.attest,
    outboxFactory: src.factory,
    feeRegistry: src.feeRegistry,
    attestorVault: src.attestorVault,
    bridgeHub: hubAddr,
  };
  out.spokes = out.spokes ?? {};
  writeFileSync(OUT, JSON.stringify(out, null, 2));
  console.log("✅ BridgeHub deployed:", hubAddr, "→", OUT);
  console.log("   next: deploy-bridge-spoke.mts <chainKey> once per spoke chain");
  process.exit(0);
}
main().catch((e) => {
  console.error("FAILED:", e.shortMessage ?? e.message);
  process.exit(1);
});
