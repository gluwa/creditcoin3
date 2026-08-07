// usc-dev smoke publish: mint/approve ATTEST → self-signed quote (deployer == authorized quoter
// EOA) → RelayerContract.publishAndCollectRelayerFee → record messageId.
// Env: DEPLOYER_KEY (payer + quoter EOA), optional MEMO.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const OUT = process.env.DEPLOY_OUT ?? "/tmp/usc-dev-deploy.json";
const a = JSON.parse(readFileSync(OUT, "utf8"));
const s = a.source;

const provider = new ethers.JsonRpcProvider(s.rpc, s.chainId, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const payer = new ethers.Wallet(process.env.DEPLOYER_KEY!, provider);

const payload = ethers.getBytes(ethers.toUtf8Bytes(process.env.MEMO ?? `usc-dev smoke ${new Date().toISOString()}`));
const payloadHash = ethers.keccak256(payload);
const now = BigInt((await provider.getBlock("latest"))!.timestamp);

// RelayerTypes.Quote — signed via EIP-191 personal_sign over the QUOTE_TYPEHASH abi-encoding.
const q = {
  coreFee: ethers.parseEther("1"),            // cap == live USCRelayingQuoter core fee
  relayPrice: ethers.parseEther("1"),         // ATTEST (payInNative=false)
  acknowledgmentPrice: ethers.parseEther("1"),// nonzero ⇒ canAck
  gasLimit: 300000n,
  destinationChain: s.chainKey,               // uint32 chain key (8)
  payloadHash,
  targetContract: payer.address,              // must equal msg.sender
  expectedCompletion: now + 600n,
  expiry: now + 3600n,
  payInNative: false,
};
const TYPEHASH = ethers.keccak256(ethers.toUtf8Bytes(
  "RelayerQuote(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bool payInNative,uint256 sourceChainId,address verifyingContract)"));
const digest = ethers.keccak256(ethers.AbiCoder.defaultAbiCoder().encode(
  ["bytes32","uint256","uint256","uint256","uint256","uint32","bytes32","address","uint256","uint256","bool","uint256","address"],
  [TYPEHASH, q.coreFee, q.relayPrice, q.acknowledgmentPrice, q.gasLimit, q.destinationChain,
   q.payloadHash, q.targetContract, q.expectedCompletion, q.expiry, q.payInNative,
   BigInt(s.chainId), s.relayerContract]));
const signature = await payer.signMessage(ethers.getBytes(digest)); // EIP-191

const signedQuote = ethers.AbiCoder.defaultAbiCoder().encode(
  ["tuple(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bool payInNative,bytes signature)"],
  [{ ...q, signature }]);

const tip = ethers.parseEther("1");
const total = q.coreFee + q.relayPrice + q.acknowledgmentPrice + tip;
const attest = new ethers.Contract(s.attest, ["function mint(address,uint256)", "function approve(address,uint256)", "function balanceOf(address) view returns (uint256)"], payer);
await (await attest.mint(payer.address, total)).wait();
await (await attest.approve(s.relayerContract, total)).wait();
console.log("minted+approved", ethers.formatEther(total), "ATTEST");

// Refresh the oracle price first: the quoter's relay-fee floor reverts StalePrice when the last
// priceUpdate is too old (the quoter service will own this in production; here the payer is the
// oracleService).
const quoter = new ethers.Contract(s.quoter,
  ["function priceUpdate(uint64,uint16,(uint256,uint256,uint256,uint256,uint16))"], payer);
const twap = new ethers.Contract(s.twapReader, ["function update(uint256)"], payer);
try {
  await (await twap.update(ethers.parseEther("1"))).wait();
  await (await quoter.priceUpdate(10_000_000_000n, s.chainKey, [0n, 1n, 10_000_000_000n, 10_000_000_000n, 0])).wait();
  console.log("refreshed TWAP + quoter price");
} catch (e) {
  // 2026-08-07: the oracle role (setOracleService) was handed to the partner's dedicated
  // oracle EOA, so this refresh reverts UnauthorizedOracle when run with the deployer key.
  // Continue anyway — the publish still works while the partner's last push is < maxPriceAge
  // (1h); if the publish then reverts StalePrice, either ask for a push or run
  // set-oracle-devnet.mts to take the role back temporarily.
  console.warn("TWAP/quoter refresh skipped (oracle role is held elsewhere):", (e as Error).message?.slice(0, 120));
}

// The Outbox requires the emitter to personally approve the RelayerContract as forwarder
// (approvedForwarders[emitter][forwarder]) — one-time, idempotent.
const outbox = new ethers.Contract(s.outbox, ["function approveForwarder(address,bool)"], payer);
await (await outbox.approveForwarder(s.relayerContract, true)).wait();
console.log("approved RelayerContract as forwarder for", payer.address);

const rc = new ethers.Contract(s.relayerContract,
  ["function publishAndCollectRelayerFee(bytes,bytes,uint256,uint256) returns (bytes32)"], payer);
const rcpt = await (await rc.publishAndCollectRelayerFee(payload, signedQuote, tip, now + 3600n)).wait();

const iface = new ethers.Interface(["event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool canAck, bytes payload)"]);
const evt = rcpt!.logs.map((l: any) => { try { return iface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "MessagePublished");
if (!evt) throw new Error("no MessagePublished event in receipt");
const messageId = evt.args.messageId as string;
writeFileSync("/tmp/usc-dev-published.json", JSON.stringify({ messageId, canAck: evt.args.canAck, block: rcpt!.blockNumber, tx: rcpt!.hash }, null, 2));
console.log("🎉 published:", messageId, "canAck:", evt.args.canAck, "block", rcpt!.blockNumber, "tx", rcpt!.hash);
process.exit(0);
