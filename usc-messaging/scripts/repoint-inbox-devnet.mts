// devnet: repoint the CREDITCOIN-side contracts of one route at a newly deployed destination Inbox.
//
// When a destination Inbox is redeployed (ABI change, e.g. asc-contracts #48 / #54) two owner calls on
// Creditcoin must follow or acks and relayer fee claims break against it:
//
//   AcknowledgmentValidator.updateTrustedInbox(newInbox, true)        // proofs of MessageExecuted
//   EVMDeliveryDecoder.setTrustedInbox(destChainId, newInbox)         // claimDelivery proof decoding
//
// The old Inbox stays trusted on the ack validator until REVOKE_OLD=true (keep it while in-flight
// pre-cutover messages can still be acknowledged). The decoder has one inbox per chain id, so the
// setter replaces the old one outright.
//
// Route selection: CHAIN_KEY picks chains.<K> in the deploy record (dest.inbox, source.outbox,
// source.ackValidator); the shared decoder is source.deliveryDecoder. Only usc-devnet's record has a
// legacy root route (source.chainKey = 8, Sepolia) that is used when CHAIN_KEY is unset.
//
// Env: NETWORK (asc-devnet default | usc-devnet), CHAIN_KEY (required unless the record has a root route),
//      DEPLOYER_KEY (owner of both contracts), NEW_INBOX, optional REVOKE_OLD=true, DEST_RPC (or
//      SEPOLIA_RPC) for the allowlist check on the destination, DEST_CHAIN_ID, CC_RPC, DEPLOY_OUT.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";
import { network, deployPath, ccProvider } from "./network.mjs";

const NET = network();
const OUT = deployPath(NET);
const CC_CHAIN_ID = NET.evmChainId;
for (const k of ["DEPLOYER_KEY", "NEW_INBOX"]) if (!process.env[k]) throw new Error(`missing ${k}`);
const NEW_INBOX = ethers.getAddress(process.env.NEW_INBOX!);
const REVOKE_OLD = (process.env.REVOKE_OLD ?? "false") === "true";

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const shared = addrs.source ?? {};
const legacyKey: number | undefined = shared.chainKey !== undefined ? Number(shared.chainKey) : undefined;
const CHAIN_KEY = process.env.CHAIN_KEY ? Number(process.env.CHAIN_KEY) : legacyKey;
if (CHAIN_KEY === undefined || !Number.isInteger(CHAIN_KEY) || CHAIN_KEY <= 0) {
  throw new Error(`missing CHAIN_KEY — ${OUT} has no legacy root route; set CHAIN_KEY to one of chains.{${Object.keys(addrs.chains ?? {}).join(",")}}`);
}
const legacyRoute = CHAIN_KEY === legacyKey && !addrs.chains?.[String(CHAIN_KEY)];
const entry = legacyRoute ? undefined : addrs.chains?.[String(CHAIN_KEY)];
const dst = legacyRoute ? addrs.dest : entry?.dest;
const src = legacyRoute ? addrs.source : entry?.source;
if (!dst?.inbox) throw new Error(`no ${legacyRoute ? "dest" : `chains.${CHAIN_KEY}.dest`}.inbox in ${OUT}`);
if (!src?.outbox || !src?.ackValidator) throw new Error(`no ${legacyRoute ? "source" : `chains.${CHAIN_KEY}.source`}.outbox / .ackValidator in ${OUT} — run deploy-source-chain-devnet first`);
if (!shared.deliveryDecoder) throw new Error(`no source.deliveryDecoder in ${OUT}`);
const DEST_CHAIN_ID = Number(process.env.DEST_CHAIN_ID ?? dst.chainId ?? entry?.destChainId ?? (legacyRoute ? 11155111 : NaN));
if (!Number.isInteger(DEST_CHAIN_ID) || DEST_CHAIN_ID <= 0) throw new Error("DEST_CHAIN_ID unknown — not recorded for this route and not set");
console.log(`route ${CHAIN_KEY} (${legacyRoute ? "legacy root" : `chains.${CHAIN_KEY}`}): dest chain id ${DEST_CHAIN_ID}, Outbox ${src.outbox}, ack validator ${src.ackValidator}`);
// First run: dest.inbox is the old Inbox. Re-run (e.g. REVOKE_OLD=true later): dest.inbox is
// already NEW_INBOX and the previous one lives in dest.oldInbox; every step below is idempotent.
const alreadyRepointed = (dst.inbox as string).toLowerCase() === NEW_INBOX.toLowerCase();
const oldInbox: string | undefined = alreadyRepointed ? dst.oldInbox : dst.inbox;
if (alreadyRepointed) console.log(`  dest.inbox is already ${NEW_INBOX}; old Inbox from dest.oldInbox: ${oldInbox ?? "none recorded"}`);
if (REVOKE_OLD && !oldInbox) throw new Error("REVOKE_OLD=true but no old Inbox is known (dest.oldInbox missing)");

const { provider } = await ccProvider(NET, shared.rpc);
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY!, provider);

// The new Inbox must allowlist our Outbox, or the relayer's first delivery reverts
// UnsupportedOutbox. Check on the destination before touching Creditcoin.
const destRpc = process.env.DEST_RPC ?? process.env.SEPOLIA_RPC;
if (destRpc) {
  const sep = new ethers.JsonRpcProvider(destRpc, DEST_CHAIN_ID, { staticNetwork: true });
  const inbox = new ethers.Contract(NEW_INBOX, [
    "function isSupportedOutbox(address) view returns (bool)",
    // #36 renamed creditcoinChainId() → sourceChainId(); the value is still the Creditcoin EVM chain id.
    "function sourceChainId() view returns (uint256)",
  ], sep);
  if (!(await inbox.isSupportedOutbox(src.outbox))) throw new Error(`${NEW_INBOX} does not allowlist Outbox ${src.outbox}; owner must setSupportedOutbox first`);
  if (Number(await inbox.sourceChainId()) !== CC_CHAIN_ID) throw new Error(`new Inbox sourceChainId != ${CC_CHAIN_ID}`);
  console.log(`  destination Inbox ${NEW_INBOX} allowlists ${src.outbox}`);
  sep.destroy();
} else {
  console.log("  DEST_RPC unset — skipping the allowlist check on the new Inbox");
}

const ack = new ethers.Contract(src.ackValidator, [
  "function owner() view returns (address)",
  "function trustedInboxes(address) view returns (bool)",
  "function updateTrustedInbox(address _inbox, bool _trusted)",
], wallet);
const decoder = new ethers.Contract(shared.deliveryDecoder, [
  "function owner() view returns (address)",
  "function trustedInboxes(uint32) view returns (address)",
  "function setTrustedInbox(uint32 destinationChainId, address inbox)",
], wallet);
for (const [name, c] of [["AcknowledgmentValidator", ack], ["EVMDeliveryDecoder", decoder]] as const) {
  const o = await c.owner();
  if (o.toLowerCase() !== wallet.address.toLowerCase()) throw new Error(`${name} owner is ${o}, not ${wallet.address}`);
}

if (!(await ack.trustedInboxes(NEW_INBOX))) {
  await (await ack.updateTrustedInbox(NEW_INBOX, true)).wait();
  console.log(`  AcknowledgmentValidator.updateTrustedInbox(${NEW_INBOX}, true)`);
} else console.log("  ack validator already trusts the new Inbox");
if (REVOKE_OLD && oldInbox) {
  if (await ack.trustedInboxes(oldInbox)) {
    await (await ack.updateTrustedInbox(oldInbox, false)).wait();
    console.log(`  AcknowledgmentValidator.updateTrustedInbox(${oldInbox}, false)`);
  } else console.log(`  old Inbox ${oldInbox} already untrusted`);
}
if ((await decoder.trustedInboxes(DEST_CHAIN_ID)).toLowerCase() !== NEW_INBOX.toLowerCase()) {
  await (await decoder.setTrustedInbox(DEST_CHAIN_ID, NEW_INBOX)).wait();
  console.log(`  EVMDeliveryDecoder.setTrustedInbox(${DEST_CHAIN_ID}, ${NEW_INBOX})`);
} else console.log("  decoder already points at the new Inbox");

const updated = {
  ...dst, inbox: NEW_INBOX, oldInbox: oldInbox ?? dst.oldInbox ?? null,
  inboxRepointedAt: alreadyRepointed ? dst.inboxRepointedAt : new Date().toISOString(),
  ...(REVOKE_OLD && oldInbox ? { oldInboxRevokedAt: new Date().toISOString() } : {}),
};
if (legacyRoute) addrs.dest = updated; else entry.dest = updated;
writeFileSync(OUT, JSON.stringify(addrs, null, 2) + "\n");
console.log(`✅ Creditcoin side repointed to ${NEW_INBOX}. Now set inboxAddress in the relayer IaC and roll attestors + relayer together.`);
process.exit(0);
