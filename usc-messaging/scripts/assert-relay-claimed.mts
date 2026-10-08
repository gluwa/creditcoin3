// Level-2 relay-fee settlement check: the relayer (started with --relayer-contract-address) auto-
// claims the funded relayFee(+tip) via RelayerContract.claimDelivery right after it acknowledges —
// AckAndClaim mode, see usc-message-relayer's ack/mod.rs module docs. This only watches for and
// verifies that outcome; unlike a manual claimant it never calls claimDelivery itself, so it can't
// race the relayer's own settlement.
import { ethers } from "ethers";
import { readFileSync } from "node:fs";

const a = JSON.parse(readFileSync("/tmp/e2e-deploy.json", "utf8"));
const pub = JSON.parse(readFileSync("/tmp/e2e-published.json", "utf8"));
const messageId: string = pub.messageId;
const src = a.source;

const provider = new ethers.JsonRpcProvider("http://127.0.0.1:9944", 42, { staticNetwork: true });
provider.pollingInterval = 800;

const rcAbi = [
  "function getMessageInfo(bytes32) view returns ((address payer,uint32 destinationChain,uint256 gasLimit,uint256 relayFee,uint256 tip,uint256 tipExpiry,uint256 deliveryDeadline,bool relaySettled,bool feesInNative))",
  "event DeliveryClaimed(bytes32 indexed messageId, address indexed relayer, address indexed submitter, uint256 relayFee, uint256 tip)",
];
const rc = new ethers.Contract(src.relayerContract, rcAbi, provider);

console.log(`  waiting for the relayer's own AckAndClaim settlement of ${messageId}…`);
let info: any;
for (let i = 0; i < 40; i++) {
  info = await rc.getMessageInfo(messageId);
  if (info.relaySettled) break;
  if (i % 5 === 0) console.log(`  not settled yet… [${i}]`);
  await new Promise((s) => setTimeout(s, 3000));
}
if (!info?.relaySettled) throw new Error(`FAIL: relay was not auto-claimed within timeout (messageId=${messageId})`);

const logs = await provider.getLogs({
  address: src.relayerContract,
  topics: [ethers.id("DeliveryClaimed(bytes32,address,address,uint256,uint256)"), messageId],
  fromBlock: 0, toBlock: "latest",
});
const iface = new ethers.Interface(rcAbi);
const claimed = logs.map((l) => { try { return iface.parseLog(l); } catch { return null; } }).find((e) => e?.name === "DeliveryClaimed");
if (!claimed) throw new Error("FAIL: relaySettled=true but no DeliveryClaimed event found");

console.log(`  DeliveryClaimed: relayer=${claimed.args.relayer} submitter=${claimed.args.submitter} relayFee=${ethers.formatEther(claimed.args.relayFee)} tip=${ethers.formatEther(claimed.args.tip)}`);

// Compare the paid amounts to the funded ledger entry, not just the event's own numbers — a lapsed
// tipExpiry would self-consistently report tip=0 in the event while still being an underpayment
// relative to what was funded.
if (claimed.args.relayFee !== info.relayFee) throw new Error(`FAIL: paid relayFee ${claimed.args.relayFee} != funded ${info.relayFee}`);
if (claimed.args.tip !== info.tip) throw new Error(`FAIL: paid tip ${claimed.args.tip} != funded ${info.tip} (tipExpiry may have lapsed — relayer underpaid)`);

console.log(`✅✅ AUTO-CLAIM PASS — relayer (${claimed.args.relayer}) paid ${ethers.formatEther(claimed.args.relayFee + claimed.args.tip)} ATTEST via its own AckAndClaim settlement, relaySettled=true`);
process.exit(0);
