// devnet: publish a message DIRECTLY on a chain key's Outbox (self-relayed path: no Lite, no
// quote, no ATTEST when the core fee is 0). The relayer still delivers it — it watches the
// Outbox, not the Lite — and the ack validator still acknowledges it when canAck is set; only
// the relayer fee claim does not apply (nothing was paid).
//
//   Outbox.publishMessage(canAck, abi.encode(destination, 0, gasLimit, memo))
//
// Env: NETWORK (asc-devnet default), CHAIN_KEY (required unless the record has a root route),
//      DEPLOYER_KEY (publisher; needs CTC), DESTINATION (default chains.<K>.dest.dapp), MEMO,
//      CAN_ACK (default true), CC_RPC, DEPLOY_OUT. Records chains.<K>.messages[] in the deploy JSON.
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";
import { memoEnvelope } from "./evm-envelope.mjs";
import { network, deployPath, ccProvider } from "./network.mjs";

const NET = network();
const OUT = deployPath(NET);
const need = (k: string): string => { const v = process.env[k]; if (!v) throw new Error(`missing ${k}`); return v; };
const deployJson = JSON.parse(readFileSync(OUT, "utf8"));
const legacyKey: number | undefined = deployJson.source?.chainKey !== undefined ? Number(deployJson.source.chainKey) : undefined;
const CHAIN_KEY = process.env.CHAIN_KEY ? Number(process.env.CHAIN_KEY) : legacyKey;
if (CHAIN_KEY === undefined || !Number.isInteger(CHAIN_KEY) || CHAIN_KEY <= 0) throw new Error(`missing CHAIN_KEY — set one of chains.{${Object.keys(deployJson.chains ?? {}).join(",")}}`);
const entry = CHAIN_KEY === legacyKey ? { source: deployJson.source, dest: deployJson.dest } : deployJson.chains?.[String(CHAIN_KEY)];
if (!entry?.source?.outbox) throw new Error(`no chains.${CHAIN_KEY}.source.outbox in ${OUT}`);
const destination = ethers.getAddress(process.env.DESTINATION ?? entry.dest?.dapp ?? (() => { throw new Error("set DESTINATION (no chains.<K>.dest.dapp recorded)"); })());
const CAN_ACK = (process.env.CAN_ACK ?? "true") === "true";
const MEMO = process.env.MEMO ?? `asc-devnet direct publish ${new Date().toISOString()}`;

const { provider } = await ccProvider(NET, deployJson.source.rpc);
const wallet = new ethers.Wallet(need("DEPLOYER_KEY"), provider);
const outbox = new ethers.Contract(entry.source.outbox, [
  "function coreFee() view returns (uint256)",
  "function publishMessage(bool canAck, bytes payload) returns (bytes32)",
  "event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, uint64 sequence, bool canAck, bytes payload)",
], wallet);
const fee: bigint = await outbox.coreFee();
console.log(`network ${NET.name} chain key ${CHAIN_KEY}: Outbox ${entry.source.outbox}, coreFee ${fee}, publisher ${wallet.address} (${ethers.formatEther(await provider.getBalance(wallet.address))} CTC)`);
if (fee !== 0n) throw new Error(`coreFee is ${fee}: the direct path would need ATTEST approval; use publish-lite-devnet.mts or set CORE_FEE_WEI=0`);
const payload = memoEnvelope(destination, MEMO);
console.log(`publishing canAck=${CAN_ACK} destination=${destination} memo="${MEMO}" envelope=${payload.length / 2 - 1} bytes`);
const tx = await outbox.publishMessage(CAN_ACK, payload);
console.log("tx", tx.hash);
const rcpt = await tx.wait();
const ev = rcpt!.logs.map((l: any) => { try { return outbox.interface.parseLog(l); } catch { return null; } }).find((e: any) => e?.name === "MessagePublished");
if (!ev) throw new Error("no MessagePublished event in the receipt");
const rec = { messageId: ev.args.messageId as string, sequence: Number(ev.args.sequence), emitter: ev.args.emitterAddress as string, canAck: CAN_ACK, destination, memo: MEMO, txHash: tx.hash, block: rcpt!.blockNumber, publishedAt: new Date().toISOString(), path: "direct" };
console.log(`✅ published messageId=${rec.messageId} sequence=${rec.sequence} block=${rec.block}`);
if (CHAIN_KEY !== legacyKey) { (deployJson.chains[String(CHAIN_KEY)].messages ??= []).push(rec); writeFileSync(OUT, JSON.stringify(deployJson, null, 2) + "\n"); console.log(`recorded under chains.${CHAIN_KEY}.messages in ${OUT}`); }
process.exit(0);
