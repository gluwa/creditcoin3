// usc-dev: repoint the CREDITCOIN-side contracts at a newly deployed Sepolia Inbox.
//
// asc-contracts #48 changed the Inbox (5-arg deliverMessage, Outbox in the attested hash), so the
// Sepolia Inbox is redeployed (Kevin, from main c83b3372 with INITIAL_OUTBOXES=<source.outbox>).
// Two owner calls on Creditcoin must follow or acks and relayer fee claims break against it:
//
//   AcknowledgmentValidator.updateTrustedInbox(newInbox, true)   // proofs of MessageDelivered
//   EVMDeliveryDecoder.setTrustedInbox(11155111, newInbox)       // claimDelivery proof decoding
//
// The old Inbox stays trusted on the ack validator until REVOKE_OLD=true (keep it while in-flight
// pre-cutover messages can still be acknowledged). The decoder has one inbox per chain id, so the
// setter replaces the old one outright.
//
// Env: DEPLOYER_KEY (owner of both contracts), NEW_INBOX, optional REVOKE_OLD=true, CC_RPC, DEPLOY_OUT.
import { ethers } from "ethers";
import { readFileSync, writeFileSync } from "node:fs";

const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;
const DEST_CHAIN_ID = 11155111;
const CC_CHAIN_ID = 42;
for (const k of ["DEPLOYER_KEY", "NEW_INBOX"]) if (!process.env[k]) throw new Error(`missing ${k}`);
const NEW_INBOX = ethers.getAddress(process.env.NEW_INBOX!);
const REVOKE_OLD = (process.env.REVOKE_OLD ?? "false") === "true";

const addrs = JSON.parse(readFileSync(OUT, "utf8"));
const s = addrs.source;
const oldInbox: string = addrs.dest.inbox;
if (oldInbox.toLowerCase() === NEW_INBOX.toLowerCase()) throw new Error(`NEW_INBOX equals the current dest.inbox ${oldInbox}`);

const provider = new ethers.JsonRpcProvider(process.env.CC_RPC ?? s.rpc, CC_CHAIN_ID, { staticNetwork: true, polling: true });
provider.pollingInterval = 1000;
const wallet = new ethers.Wallet(process.env.DEPLOYER_KEY!, provider);

// The new Inbox must be the #48 shape and must allowlist our Outbox, or the relayer's first
// delivery reverts UnsupportedOutbox. Check on Sepolia before touching Creditcoin.
if (process.env.SEPOLIA_RPC) {
  const sep = new ethers.JsonRpcProvider(process.env.SEPOLIA_RPC, DEST_CHAIN_ID, { staticNetwork: true });
  const inbox = new ethers.Contract(NEW_INBOX, [
    "function isSupportedOutbox(address) view returns (bool)",
    "function creditcoinChainId() view returns (uint256)",
  ], sep);
  if (!(await inbox.isSupportedOutbox(s.outbox))) throw new Error(`${NEW_INBOX} does not allowlist Outbox ${s.outbox}; owner must setSupportedOutbox first`);
  if (Number(await inbox.creditcoinChainId()) !== CC_CHAIN_ID) throw new Error("new Inbox creditcoinChainId != 42");
  console.log(`  Sepolia Inbox ${NEW_INBOX} allowlists ${s.outbox}`);
  sep.destroy();
} else {
  console.log("  SEPOLIA_RPC unset — skipping the allowlist check on the new Inbox");
}

const ack = new ethers.Contract(s.ackValidator, [
  "function owner() view returns (address)",
  "function trustedInboxes(address) view returns (bool)",
  "function updateTrustedInbox(address _inbox, bool _trusted)",
], wallet);
const decoder = new ethers.Contract(s.deliveryDecoder, [
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
if (REVOKE_OLD && (await ack.trustedInboxes(oldInbox))) {
  await (await ack.updateTrustedInbox(oldInbox, false)).wait();
  console.log(`  AcknowledgmentValidator.updateTrustedInbox(${oldInbox}, false)`);
}
if ((await decoder.trustedInboxes(DEST_CHAIN_ID)).toLowerCase() !== NEW_INBOX.toLowerCase()) {
  await (await decoder.setTrustedInbox(DEST_CHAIN_ID, NEW_INBOX)).wait();
  console.log(`  EVMDeliveryDecoder.setTrustedInbox(${DEST_CHAIN_ID}, ${NEW_INBOX})`);
} else console.log("  decoder already points at the new Inbox");

addrs.dest = { ...addrs.dest, inbox: NEW_INBOX, oldInbox, inboxRepointedAt: new Date().toISOString() };
writeFileSync(OUT, JSON.stringify(addrs, null, 2));
console.log(`✅ Creditcoin side repointed to ${NEW_INBOX}. Now set inboxAddress in the relayer IaC and roll attestors + relayer together.`);
process.exit(0);
