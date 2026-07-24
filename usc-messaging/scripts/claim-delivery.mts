// Level-2 relayer payout: prove the destination MessageDelivered tx via proof-gen and call
// RelayerFeeVault.claimDelivery on the source (CC EVM). The claim is permissionless — msg.sender
// (a dedicated claimant EOA here, funded with gas only) receives the funded relayFee (+tip).
//
// Proof bundle comes from the same proof-gen `proof-by-tx` endpoint the ack path uses; it is only
// available once the destination block is attested on CC3 (i.e. after the ack round-trip lands).
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const PROOF_GEN = process.env.PROOF_GEN_URL ?? "http://127.0.0.1:3100";
const DEST_CHAIN_KEY = 2; // anvil destination — proof-gen path param + claimDelivery chainKey
const BALTHATHAR = "0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b";
// Dedicated claimant (anvil #8 key, reused as a plain CC-EVM keypair): starts with 0 ATTEST so the
// payout delta is exactly relayFee (+tip). Gas is topped up from Balthathar below.
const CLAIMANT_KEY = "0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97";

const a = JSON.parse(readFileSync("/tmp/e2e-deploy.json", "utf8"));
const pub = JSON.parse(readFileSync("/tmp/e2e-published.json", "utf8"));
const messageId: string = pub.messageId;
const src = a.source;
const dest = a.dest;

const srcProvider = new ethers.JsonRpcProvider("http://127.0.0.1:9944", 42, { staticNetwork: true });
srcProvider.pollingInterval = 800;
const destProvider = new ethers.JsonRpcProvider("http://127.0.0.1:8545", 31337, { staticNetwork: true });
destProvider.pollingInterval = 500;

const funder = new ethers.Wallet(BALTHATHAR, srcProvider);
const claimant = new ethers.Wallet(CLAIMANT_KEY, srcProvider);

// ── 1. Find the destination delivery tx (Inbox.MessageDelivered for this messageId) ──────────────
const deliveredSig = ethers.id("MessageDelivered(bytes32,address,address)");
const logs = await destProvider.getLogs({
  address: dest.inbox,
  topics: [deliveredSig, messageId],
  fromBlock: 0,
  toBlock: "latest",
});
if (logs.length === 0) throw new Error(`no MessageDelivered for ${messageId} on Inbox ${dest.inbox}`);
const deliveryTx = logs[0].transactionHash;
console.log("  delivery tx (dest):", deliveryTx);

// ── 2. Fetch the native USC proof for that tx (retry while the block is still being attested) ────
type ProofResp = {
  headerNumber: number;
  txBytes: string;
  continuityProof: { lowerEndpointDigest: string; roots: string[] };
  merkleProof: { root: string; siblings: { hash: string; isLeft: boolean }[] };
};
let proof: ProofResp | undefined;
for (let i = 0; i < 60; i++) {
  const r = await fetch(`${PROOF_GEN}/api/v1/proof-by-tx/${DEST_CHAIN_KEY}/${deliveryTx}`);
  if (r.status === 422) { if (i % 5 === 0) console.log(`  proof not ready (422), waiting… [${i}]`); await new Promise((s) => setTimeout(s, 3000)); continue; }
  if (!r.ok) throw new Error(`proof-gen ${r.status}: ${await r.text()}`);
  proof = (await r.json()) as ProofResp;
  break;
}
if (!proof || !proof.txBytes) throw new Error("proof-gen never returned a usable proof (no txBytes)");
console.log("  proof ready: headerNumber", proof.headerNumber, "siblings", proof.merkleProof.siblings.length);

// ── 3. Build the BlockProverTypes structs claimDelivery expects ──────────────────────────────────
// InclusionProof.data = abi.encode(bytes txBytes, MerkleProofEntry[] siblings), entry {bytes32 sibling, bool isLeft}.
const data = ethers.AbiCoder.defaultAbiCoder().encode(
  ["bytes", "tuple(bytes32 sibling, bool isLeft)[]"],
  [proof.txBytes, proof.merkleProof.siblings.map((s) => ({ sibling: s.hash, isLeft: s.isLeft }))],
);
const inclusionProof = { kind: 0, root: proof.merkleProof.root, data }; // ProofKind.BinaryMerkle = 0
const continuityProof = { lowerEndpointDigest: proof.continuityProof.lowerEndpointDigest, roots: proof.continuityProof.roots };
const chainKey = ethers.zeroPadValue(ethers.toBeHex(DEST_CHAIN_KEY), 32); // bytes32(uint256(destinationChain))

// ── 4. Top up claimant gas, snapshot balances, claim ─────────────────────────────────────────────
const attest = new ethers.Contract(src.attest, ["function balanceOf(address) view returns (uint256)"], srcProvider);
const fvAbi = [
  "function getMessageInfo(bytes32) view returns ((address payer,uint32 destinationChain,uint256 gasLimit,uint256 relayFee,uint256 tip,uint256 tipExpiry,uint256 deliveryDeadline,bool relaySettled))",
  "function claimDelivery(bytes32 messageId, bytes32 chainKey, uint64 blockHeight, (uint8 kind, bytes32 root, bytes data) inclusionProof, (bytes32 lowerEndpointDigest, bytes32[] roots) continuityProof)",
  "event DeliveryClaimed(bytes32 indexed messageId, address indexed relayer, uint256 relayFee, uint256 tip)",
];
const fv = new ethers.Contract(src.relayerFeeVault, fvAbi, claimant);

const infoBefore = await fv.getMessageInfo(messageId);
if (infoBefore.relaySettled) throw new Error("relay already settled before claim — nothing to prove");
console.log("  funded relayFee:", ethers.formatEther(infoBefore.relayFee), "tip:", ethers.formatEther(infoBefore.tip), "relaySettled:", infoBefore.relaySettled);

await (await funder.sendTransaction({ to: claimant.address, value: ethers.parseEther("1") })).wait(); // gas
const balBefore = await attest.balanceOf(claimant.address);

const rcpt = await (await fv.claimDelivery(messageId, chainKey, proof.headerNumber, inclusionProof, continuityProof)).wait();
const claimed = rcpt.logs.map((l: any) => { try { return fv.interface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "DeliveryClaimed");

const balAfter = await attest.balanceOf(claimant.address);
const infoAfter = await fv.getMessageInfo(messageId);
const delta = balAfter - balBefore;

// ── 5. Assert the payout ─────────────────────────────────────────────────────────────────────────
console.log("  DeliveryClaimed:", claimed ? `relayFee=${ethers.formatEther(claimed.args.relayFee)} tip=${ethers.formatEther(claimed.args.tip)}` : "MISSING");
console.log("  claimant ATTEST delta:", ethers.formatEther(delta), "relaySettled:", infoAfter.relaySettled);

if (!claimed) throw new Error("FAIL: no DeliveryClaimed event");
if (!infoAfter.relaySettled) throw new Error("FAIL: relaySettled did not flip to true");
const expected = claimed.args.relayFee + claimed.args.tip;
if (delta !== expected) throw new Error(`FAIL: ATTEST delta ${delta} != relayFee+tip ${expected}`);
if (delta <= 0n) throw new Error("FAIL: claimant balance did not increase");

console.log(`✅✅ CLAIM PASS — relayer paid ${ethers.formatEther(delta)} ATTEST, relaySettled=true`);
process.exit(0);
