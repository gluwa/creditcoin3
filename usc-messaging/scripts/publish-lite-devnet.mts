// usc-dev paid publish through RelayerContractLite (the canonical devnet relayer contract since
// 2026-09-04): a DEDICATED quoter EOA signs the quote, the deployer pays and publishes.
//
//   payer  (DEPLOYER_KEY)     — holds ATTEST + CTC, is `msg.sender`, so `Quote.targetContract`
//   quoter (QUOTER_TEST_KEY)  — only signs; must be in `RelayerContractLite.authorizedQuoters`.
//                                Never the shared deployer: nothing floors a price on Lite, so any
//                                authorised key can sign a fee the vault settles.
//
// Lite differences vs. publish-devnet.mts (legacy RelayerContract 0x115f…):
//   * publishAndCollectRelayerFee(bytes payload, bytes signedQuote) — two args, no tip / deadline
//   * no on-chain quoter / TWAP to refresh; the quote's coreFee only has to be >= outbox.coreFee()
//   * fees: coreFee + acknowledgmentPrice (+ relayPrice when !payInNative) pulled in ATTEST
//
// Env: DEPLOYER_KEY, QUOTER_TEST_KEY, optional MEMO, REQUIRES_ACK=false, DEPLOY_OUT.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";
import { memoEnvelope } from "./evm-envelope.mjs";

const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const deployJson = JSON.parse(readFileSync(OUT, "utf8"));
// CHAIN_KEY selects a secondary destination registered by register-chain-devnet / deploy-*-chain-devnet
// (e.g. 9 = Base Sepolia): its per-chain source stack (Outbox, Lite, vault) overlays the shared
// `source` fields (rpc, chainId, attest, …). Unset = the original Sepolia route (chain key 8).
const CHAIN_KEY_SEL = process.env.CHAIN_KEY ? Number(process.env.CHAIN_KEY) : undefined;
const chainEntry = CHAIN_KEY_SEL !== undefined && CHAIN_KEY_SEL !== Number(deployJson.source.chainKey)
  ? deployJson.chains?.[String(CHAIN_KEY_SEL)] : undefined;
if (CHAIN_KEY_SEL !== undefined && CHAIN_KEY_SEL !== Number(deployJson.source.chainKey) && !chainEntry?.source) {
  throw new Error(`no chains.${CHAIN_KEY_SEL}.source in the deploy JSON — run deploy-source-chain-devnet first`);
}
const s = chainEntry ? { ...deployJson.source, ...chainEntry.source, chainKey: CHAIN_KEY_SEL } : deployJson.source;
// Envelope destination: the MockDestination behind the #36 DispatcherRouter (override with DESTINATION).
const destination: string | undefined = process.env.DESTINATION ?? (chainEntry ? chainEntry.dest?.dapp : deployJson.dest?.dapp);
if (!destination) throw new Error("need dest.dapp in the deploy JSON (or DESTINATION) — the #36 envelope needs a destination contract");
if ((s.relayerContractKind ?? "") !== "RelayerContractLite") {
  throw new Error(`source.relayerContractKind is ${s.relayerContractKind ?? "unset"}; this script is for RelayerContractLite`);
}
for (const k of ["DEPLOYER_KEY", "QUOTER_TEST_KEY"]) if (!process.env[k]) throw new Error(`missing ${k}`);

const provider = new ethers.JsonRpcProvider(s.rpc, s.chainId, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const payer = new ethers.Wallet(process.env.DEPLOYER_KEY!, provider);
const quoter = new ethers.Wallet(process.env.QUOTER_TEST_KEY!); // signs only, never sends
const requiresAck = (process.env.REQUIRES_ACK ?? "false") === "true";

const lite = new ethers.Contract(s.relayerContract, [
  "function publishAndCollectRelayerFee(bytes payload, bytes signedQuote) payable returns (bytes32)",
  "function authorizedQuoters(address) view returns (bool)",
  "function getMessageInfo(bytes32) view returns (address payer,uint32 destinationChain,uint256 gasLimit,uint256 relayFee,uint256 tip,uint256 tipExpiry,uint256 deliveryDeadline,bool relaySettled,bool feesInNative)",
  "event FeeCollected(bytes32 indexed messageId, address indexed payer, uint256 relayPrice, uint256 acknowledgmentPrice, uint256 gasLimit, uint32 destinationChain)",
], payer);
const outbox = new ethers.Contract(s.outbox, [
  "function coreFee() view returns (uint256)",
  "function chainKey() view returns (uint32)",
  "function approveForwarder(address forwarder, bool approved)",
  "event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool canAck, bytes payload)",
], payer);
const attest = new ethers.Contract(s.attest, [
  "function balanceOf(address) view returns (uint256)",
  "function allowance(address,address) view returns (uint256)",
  "function approve(address,uint256)",
], payer);

if (!(await lite.authorizedQuoters(quoter.address))) {
  throw new Error(`${quoter.address} is not an authorised quoter on ${s.relayerContract}; ask the Lite owner for setAuthorizedQuoter`);
}
const chainKey = Number(await outbox.chainKey());
const liveCoreFee: bigint = await outbox.coreFee();
const now = BigInt((await provider.getBlock("latest"))!.timestamp);

// asc-contracts #36 envelope: abi.encode(destination, nativeCoinValue = 0, gasLimit, utf8 memo). The
// DispatcherRouter decodes it on the destination and calls `destination` with memo ++ bytes20(emitter).
// payloadHash hashes the FULL envelope — that is what the Outbox stores and the relayer forwards.
const payload = ethers.getBytes(memoEnvelope(destination, process.env.MEMO ?? `usc-dev lite smoke ${new Date().toISOString()}`));
const payloadHash = ethers.keccak256(payload);

// RelayerTypes.Quote. coreFee is a CAP the contract checks against outbox.coreFee() at publish; it
// still pulls only the live value, so a little headroom costs nothing and survives a fee bump.
const q = {
  coreFee: liveCoreFee * 2n,
  relayPrice: ethers.parseEther("1"),                                   // ATTEST (payInNative=false)
  acknowledgmentPrice: requiresAck ? ethers.parseEther("1") : 0n,       // nonzero ⇒ Outbox canAck
  gasLimit: 500000n,                                                     // funds the whole deliverMessage tx (votes + router hop + 200k envelope budget); relayer pins to it
  destinationChain: chainKey,
  payloadHash,
  targetContract: payer.address,                                        // == msg.sender
  expectedCompletion: now + 600n,                                       // becomes the refund deadline
  expiry: now + 3600n,
  payInNative: false,
};

// Digest exactly as RelayerFeeLedger._validateQuote: keccak(abi.encode(TYPEHASH, fields…, chainid, address(this))),
// then EIP-191 personal_sign — deliberately not EIP-712.
const TYPEHASH = ethers.keccak256(ethers.toUtf8Bytes(
  "RelayerQuote(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bool payInNative,uint256 sourceChainId,address verifyingContract)"));
const digest = ethers.keccak256(ethers.AbiCoder.defaultAbiCoder().encode(
  ["bytes32","uint256","uint256","uint256","uint256","uint32","bytes32","address","uint256","uint256","bool","uint256","address"],
  [TYPEHASH, q.coreFee, q.relayPrice, q.acknowledgmentPrice, q.gasLimit, q.destinationChain,
   q.payloadHash, q.targetContract, q.expectedCompletion, q.expiry, q.payInNative,
   BigInt(s.chainId), s.relayerContract]));
const signature = await quoter.signMessage(ethers.getBytes(digest));
const signedQuote = ethers.AbiCoder.defaultAbiCoder().encode(
  ["tuple(uint256 coreFee,uint256 relayPrice,uint256 acknowledgmentPrice,uint256 gasLimit,uint32 destinationChain,bytes32 payloadHash,address targetContract,uint256 expectedCompletion,uint256 expiry,bool payInNative,bytes signature)"],
  [{ ...q, signature }]);

// ATTEST the contract will pull: live coreFee + ack + relay (payInNative=false).
const total = liveCoreFee + q.acknowledgmentPrice + q.relayPrice;
const bal: bigint = await attest.balanceOf(payer.address);
if (bal < total) throw new Error(`payer has ${ethers.formatEther(bal)} ATTEST, needs ${ethers.formatEther(total)}`);
if ((await attest.allowance(payer.address, s.relayerContract)) < total) {
  await (await attest.approve(s.relayerContract, total)).wait();
  console.log("approved", ethers.formatEther(total), "ATTEST to RelayerContractLite");
}
// The Outbox only accepts publishMessageFrom for emitters that approved the forwarder; idempotent.
await (await outbox.approveForwarder(s.relayerContract, true)).wait();

console.log(`payer ${payer.address}  quoter ${quoter.address}  lite ${s.relayerContract}`);
console.log(`coreFee(live) ${liveCoreFee}  relayPrice ${ethers.formatEther(q.relayPrice)} ATTEST  ack ${ethers.formatEther(q.acknowledgmentPrice)} ATTEST  requiresAck=${requiresAck}`);
const rcpt = await (await lite.publishAndCollectRelayerFee(payload, signedQuote)).wait();

const parse = (iface: ethers.Interface, name: string) =>
  rcpt!.logs.map((l: any) => { try { return iface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === name);
const published = parse(outbox.interface, "MessagePublished");
const collected = parse(lite.interface, "FeeCollected");
if (!published) throw new Error("no MessagePublished event in receipt");
const messageId = published.args.messageId as string;
const info = await lite.getMessageInfo(messageId);
const record = {
  messageId, canAck: published.args.canAck, block: rcpt!.blockNumber, tx: rcpt!.hash,
  relayerContract: s.relayerContract, quoter: quoter.address, payer: payer.address,
  relayFee: collected ? collected.args.relayPrice.toString() : null,
  deliveryDeadline: info.deliveryDeadline.toString(), relaySettled: info.relaySettled,
};
writeFileSync("/tmp/usc-dev-published.json", JSON.stringify(record, null, 2));
console.log("🎉 published via Lite:", record);
process.exit(0);
