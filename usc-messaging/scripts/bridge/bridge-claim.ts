// Anvil → Creditcoin bridge claim: fetch a native USC proof for an AnvilBridge.lock tx from the
// proof-gen API and submit it to CcBridge.claim on Creditcoin. Mirrors the Rust ack submitter, but
// one-shot for the demo.
//
// Usage: tsx bridge-claim.ts --lock-tx 0x<anvil lock tx hash>
// Env: CREDITCOIN_RPC_URL, CC_BRIDGE_ADDR, BRIDGE_CLAIM_KEY (funded CC EVM key),
//      PROOF_GEN_URL (default http://127.0.0.1:3100), ANVIL_CHAIN_KEY (default 2).
import "dotenv/config";
import { ethers } from "ethers";

function arg(name: string): string | undefined {
  const i = process.argv.indexOf(name);
  return i !== -1 && i + 1 < process.argv.length ? process.argv[i + 1] : undefined;
}

const LOCK_TX = arg("--lock-tx");
const RPC = process.env.CREDITCOIN_RPC_URL ?? "http://127.0.0.1:9944";
const CC_BRIDGE = process.env.CC_BRIDGE_ADDR;
const KEY = process.env.BRIDGE_CLAIM_KEY ?? process.env.CREDITCOIN_CHAIN_PRIVATE_KEY;
const PROOF_GEN = process.env.PROOF_GEN_URL ?? "http://127.0.0.1:3100";
const CHAIN_KEY = process.env.ANVIL_CHAIN_KEY ?? "2";

if (!LOCK_TX) throw new Error("missing --lock-tx");
if (!CC_BRIDGE) throw new Error("missing CC_BRIDGE_ADDR");
if (!KEY) throw new Error("missing BRIDGE_CLAIM_KEY / CREDITCOIN_CHAIN_PRIVATE_KEY");

const CLAIM_ABI = [
  "function claim(uint64 height, bytes encodedTransaction, (bytes32 root, (bytes32 hash, bool isLeft)[] siblings) merkleProof, (bytes32 lowerEndpointDigest, bytes32[] roots) continuityProof) external",
  "event Claimed(address indexed ccRecipient, uint256 amount, uint256 nonce)",
];

interface ProofResponse {
  headerNumber: number;
  txBytes: string | null;
  continuityProof: { lowerEndpointDigest: string; roots: string[] };
  merkleProof: { root: string; siblings: { hash: string; isLeft: boolean }[] };
}

async function fetchProof(): Promise<ProofResponse> {
  const url = `${PROOF_GEN.replace(/\/$/, "")}/api/v1/proof-by-tx/${CHAIN_KEY}/${LOCK_TX}`;
  // The Anvil block holding the lock must be attested on CC before a proof exists. Early in a run
  // proof-gen reports this as 422 BlockNotReady OR 404 AttestationsMissing (no chain-2 attestations
  // yet) — both are transient at startup, so retry through them until the pipeline catches up.
  for (let attempt = 1; attempt <= 100; attempt++) {
    const resp = await fetch(url);
    if (!resp.ok) {
      const body = await resp.text();
      const transient =
        resp.status === 422 ||
        body.includes("BlockNotReady") ||
        body.includes("AttestationsMissing");
      if (transient) {
        if (attempt % 5 === 0) {
          console.log(`⏳ proof not ready (${resp.status} ${body.slice(0, 60)}), attempt ${attempt}…`);
        }
        await new Promise((r) => setTimeout(r, 3000));
        continue;
      }
      throw new Error(`proof-gen ${resp.status}: ${body}`);
    }
    const proof = (await resp.json()) as ProofResponse;
    if (!proof.txBytes) throw new Error("proof-gen returned no txBytes (continuity-only proof)");
    return proof;
  }
  throw new Error("timed out waiting for proof-gen");
}

async function main() {
  console.log(`🔎 fetching proof for lock tx ${LOCK_TX} (chain_key ${CHAIN_KEY})…`);
  const proof = await fetchProof();
  console.log(`✅ proof ready at height ${proof.headerNumber}`);

  const provider = new ethers.JsonRpcProvider(RPC);
  const wallet = new ethers.Wallet(KEY!, provider);
  const bridge = new ethers.Contract(CC_BRIDGE!, CLAIM_ABI, wallet);

  const merkleProof = {
    root: proof.merkleProof.root,
    siblings: proof.merkleProof.siblings.map((s) => ({ hash: s.hash, isLeft: s.isLeft })),
  };
  const continuityProof = {
    lowerEndpointDigest: proof.continuityProof.lowerEndpointDigest,
    roots: proof.continuityProof.roots,
  };

  console.log("📤 submitting CcBridge.claim…");
  const tx = await bridge.claim(proof.headerNumber, proof.txBytes, merkleProof, continuityProof);
  console.log("tx:", tx.hash);
  const receipt = await tx.wait();
  console.log(`✅ claim confirmed in block ${receipt.blockNumber} (status ${receipt.status})`);
}

main().catch((err) => {
  console.error("❌ claim failed:", err.message ?? err);
  process.exit(1);
});
