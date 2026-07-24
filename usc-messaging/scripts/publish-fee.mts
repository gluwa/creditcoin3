// Plain-ethers fee-publish: quote from the live quoter → approve → publishAndCollectRelayerFee.
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const a = JSON.parse(readFileSync("/tmp/e2e-deploy.json", "utf8"));
const s = a.source;
const provider = new ethers.JsonRpcProvider("http://127.0.0.1:9944", 42, { staticNetwork: true });
provider.pollingInterval = 800;
const payer = new ethers.Wallet("0x8075991ce870b93a8870eca0c0f91913d12f47948ca0fd25b49c6fa7cdbeee8b", provider);

// SimpleInbox.deliverMessage abi-decodes the payload as (address destinationContract, bytes data).
const inner = ethers.toUtf8Bytes(`e2e delivery ${process.argv[2] ?? "1"}`);
const payload = ethers.getBytes(ethers.AbiCoder.defaultAbiCoder().encode(["address", "bytes"], [a.dest.dapp, inner]));
const res = await fetch("http://localhost:3010/quote", {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ destinationChain: s.chainKey, targetContract: payer.address, payloadHash: ethers.keccak256(payload), gasLimit: 300000, requiresAck: true }),
});
const body = await res.json();
if (!body.signedQuote) throw new Error("quoter: " + JSON.stringify(body));
const total = BigInt(body.quote.coreFee) + BigInt(body.quote.relayPrice) + BigInt(body.quote.acknowledgmentPrice) + ethers.parseEther("5");

const attest = new ethers.Contract(s.attest, ["function mint(address,uint256)", "function approve(address,uint256)"], payer);
await (await attest.mint(payer.address, total)).wait();
await (await attest.approve(s.relayerContract, total)).wait();

const rc = new ethers.Contract(s.relayerContract, ["function publishAndCollectRelayerFee(bool,bytes,bytes,uint256,uint256) returns (bytes32)"], payer);
const now = BigInt((await provider.getBlock("latest"))!.timestamp);
const rcpt = await (await rc.publishAndCollectRelayerFee(true, payload, body.signedQuote, ethers.parseEther("5"), now + 3600n)).wait();
const iface = new ethers.Interface(["event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool requiresAck, bytes payload)"]);
const evt = rcpt!.logs.map((l: any) => { try { return iface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "MessagePublished");
console.log("🎉 published:", evt!.args.messageId, "block", rcpt!.blockNumber);
process.exit(0);
