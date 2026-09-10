// Permissionless relay-fee claim on usc-devnet for one delivered message: fetch the proof-gen proof of
// the destination delivery tx and call RelayerContractLite.claimDelivery on Creditcoin.
//
// Use when the relayer gave up on a claim (it classifies a decoder revert as terminal and never
// retries) and the delivery is already older than its restart rewind. msg.sender receives the funded
// relayFee (+tip), so the claimant here is the same submitter EOA the relayer uses (DEPLOYER_KEY).
//
// Env: CHAIN_KEY (8 = Sepolia route in source/dest, else chains.<K>), MESSAGE_ID, DELIVERY_TX,
//      PROOF_GEN_URL (default http://127.0.0.1:3101 — `kubectl port-forward svc/cc3-usc-dev-proof-gen-api 3101:3100`),
//      DEPLOYER_KEY or CLAIMANT_KEY, ASC_CONTRACTS_DIR (compiled main c83b3372+), optional CC_RPC, DEPLOY_OUT,
//      DRY_RUN=true to stop after the eth_call simulation.
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const UC = process.env.ASC_CONTRACTS_DIR ?? process.env.USC_CONTRACTS_DIR;
if (!UC) throw new Error("set ASC_CONTRACTS_DIR to a compiled asc-contracts checkout (main c83b3372+)");
const ART = (p: string, n: string) => JSON.parse(readFileSync(`${UC}/artifacts/contracts/${p}/${n}.json`, "utf8"));
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const need = (k: string): string => { const v = process.env[k]; if (!v) throw new Error(`missing ${k}`); return v; };
const CHAIN_KEY = Number(process.env.CHAIN_KEY ?? 8);
const MESSAGE_ID = ethers.hexlify(need("MESSAGE_ID"));
const DELIVERY_TX = ethers.hexlify(need("DELIVERY_TX"));
const PROOF_GEN = process.env.PROOF_GEN_URL ?? "http://127.0.0.1:3101";
const KEY = process.env.CLAIMANT_KEY ?? need("DEPLOYER_KEY");
const DRY_RUN = (process.env.DRY_RUN ?? "false") === "true";

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const src = CHAIN_KEY === 8 ? addrs.source : addrs.chains?.[String(CHAIN_KEY)]?.source;
if (!src?.relayerContract) throw new Error(`no source.relayerContract for chain key ${CHAIN_KEY} in ${OUT}`);
const provider = new ethers.JsonRpcProvider(process.env.CC_RPC ?? addrs.source.rpc, 42, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const claimant = new ethers.Wallet(KEY, provider);
const rc = new ethers.Contract(src.relayerContract, ART("write-ability/RelayerContractLite.sol", "RelayerContractLite").abi, claimant);
console.log(`route ${CHAIN_KEY}: Lite ${src.relayerContract} decoder ${await rc.deliveryDecoder()} claimant ${claimant.address}`);

const info = await rc.getMessageInfo(MESSAGE_ID);
if (info.relaySettled) { console.log("relay already settled — nothing to claim"); process.exit(0); }
if (Number(info.destinationChain) !== CHAIN_KEY) throw new Error(`message destinationChain ${info.destinationChain} != CHAIN_KEY ${CHAIN_KEY}`);
const chainKey = ethers.zeroPadValue(ethers.toBeHex(CHAIN_KEY), 32);
console.log("  funded relayFee:", ethers.formatEther(info.relayFee), "tip:", ethers.formatEther(info.tip), "feesInNative:", info.feesInNative);

type ProofResp = {
  headerNumber: number; txBytes: string | null;
  continuityProof: { lowerEndpointDigest: string; roots: string[] };
  merkleProof: { root: string; siblings: { hash: string; isLeft: boolean }[] };
};
const r = await fetch(`${PROOF_GEN}/api/v1/proof-by-tx/${CHAIN_KEY}/${DELIVERY_TX}`);
if (!r.ok) throw new Error(`proof-gen ${r.status}: ${await r.text()}`);
const proof = (await r.json()) as ProofResp;
if (!proof.txBytes) throw new Error("proof-gen returned a continuity-only proof (no txBytes) — destination block not attested yet");
console.log("  proof: headerNumber", proof.headerNumber, "siblings", proof.merkleProof.siblings.length, "roots", proof.continuityProof.roots.length);
const data = ethers.AbiCoder.defaultAbiCoder().encode(
  ["bytes", "tuple(bytes32 sibling, bool isLeft)[]"],
  [proof.txBytes, proof.merkleProof.siblings.map((s) => ({ sibling: s.hash, isLeft: s.isLeft }))],
);
const inclusionProof = { kind: 0, root: proof.merkleProof.root, data }; // ProofKind.BinaryMerkle
const continuityProof = { lowerEndpointDigest: proof.continuityProof.lowerEndpointDigest, roots: proof.continuityProof.roots };

try {
  await rc.claimDelivery.staticCall(MESSAGE_ID, chainKey, proof.headerNumber, inclusionProof, continuityProof);
  console.log("  eth_call claimDelivery: OK");
} catch (e: any) {
  const d = e?.data ?? e?.info?.error?.data ?? "";
  let name = "";
  try { name = rc.interface.parseError(d)?.name ?? ""; } catch {}
  throw new Error(`claimDelivery would revert: ${name || d || e?.shortMessage || e}`);
}
if (DRY_RUN) { console.log("DRY_RUN — not sending"); process.exit(0); }

const rcpt = await (await rc.claimDelivery(MESSAGE_ID, chainKey, proof.headerNumber, inclusionProof, continuityProof)).wait();
const claimed = rcpt.logs.map((l: any) => { try { return rc.interface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "DeliveryClaimed");
const after = await rc.getMessageInfo(MESSAGE_ID);
console.log("  tx:", rcpt.hash, "gasUsed:", rcpt.gasUsed.toString());
console.log("  DeliveryClaimed:", claimed ? `relayer=${claimed.args.relayer} submitter=${claimed.args.submitter} relayFee=${ethers.formatEther(claimed.args.relayFee)} tip=${ethers.formatEther(claimed.args.tip)}` : "MISSING");
if (!claimed || !after.relaySettled) throw new Error("FAIL: claim did not settle");
console.log(`✅ CLAIM PASS — route ${CHAIN_KEY} message ${MESSAGE_ID} relaySettled=true`);
process.exit(0);
