// Plain-ethers fee-publish: quote from the live quoter → approve → publishAndCollectRelayerFee.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const a = JSON.parse(readFileSync("/tmp/e2e-deploy.json", "utf8"));
const s = a.source;
const provider = new ethers.JsonRpcProvider("http://127.0.0.1:9944", 42, { staticNetwork: true });
provider.pollingInterval = 800;
const payer = new ethers.Wallet("0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b", provider);

// Post-#23: Inbox always dispatches to its fixed messageDispatcher (the dApp, wired at deploy
// time) — the payload is opaque dApp-specific data, no longer (address destinationContract, bytes).
const payload = ethers.toUtf8Bytes(`e2e delivery ${process.argv[2] ?? "1"}`);

// Do every on-chain prerequisite BEFORE fetching the quote: the quote's expectedCompletion/expiry
// clock starts ticking at fetch time, and each of these is a separate mined transaction — fetching
// the quote first (as this script used to) let their cumulative wait eat into that short window,
// intermittently expiring the quote before publishAndCollectRelayerFee ever ran. A generous fixed
// mint/approve (rather than the exact quoted total) means neither depends on the quote either.
const BUFFER = ethers.parseEther("1000");
const attest = new ethers.Contract(s.attest, ["function mint(address,uint256)", "function approve(address,uint256)"], payer);
await (await attest.mint(payer.address, BUFFER)).wait();
await (await attest.approve(s.relayerContract, BUFFER)).wait();

// New in the post-#23 Outbox: a trusted forwarder (RelayerContract) alone cannot attribute
// messages to an emitter that never opted in — the payer must approve it explicitly first.
const outbox = new ethers.Contract(s.outbox, ["function approveForwarder(address,bool)"], payer);
await (await outbox.approveForwarder(s.relayerContract, true)).wait();

const res = await fetch("http://localhost:3010/quote", {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ destinationChain: s.chainKey, targetContract: payer.address, payloadHash: ethers.keccak256(payload), gasLimit: 300000, requiresAck: true }),
});
const body = await res.json();
if (!body.signedQuote) throw new Error("quoter: " + JSON.stringify(body));

// v3: no requiresAck argument — a nonzero quoted acknowledgmentPrice IS the ack request.
const rc = new ethers.Contract(s.relayerContract, ["function publishAndCollectRelayerFee(bytes,bytes,uint256,uint256) returns (bytes32)"], payer);
const now = BigInt((await provider.getBlock("latest"))!.timestamp);
const rcpt = await (await rc.publishAndCollectRelayerFee(payload, body.signedQuote, ethers.parseEther("5"), now + 3600n)).wait();
const iface = new ethers.Interface(["event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool canAck, bytes payload)"]);
const evt = rcpt!.logs.map((l: any) => { try { return iface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "MessagePublished");
const messageId = evt!.args.messageId as string;
// Persist for the ack + claimDelivery assertions (they key off the exact messageId).
writeFileSync("/tmp/e2e-published.json", JSON.stringify({ messageId, block: rcpt!.blockNumber }, null, 2));
console.log("🎉 published:", messageId, "block", rcpt!.blockNumber);
process.exit(0);
