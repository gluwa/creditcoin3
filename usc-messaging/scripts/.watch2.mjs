import { ethers } from "ethers";
import { readFileSync } from "node:fs";
const ASC = "/private/tmp/claude-501/-Users-dylan-Projects-creditcoin3/7ea76496-06e9-4b09-97b2-3d23c97d60bc/scratchpad/asc-contracts-main";
const A = (p) => JSON.parse(readFileSync(`${ASC}/artifacts/contracts/${p}.json`, "utf8")).abi;
const P = (u, id) => new ethers.JsonRpcProvider(u, id, { staticNetwork: true });
const cc = P("https://rpc.asc-devnet.creditcoin.network", 102037);
const R = {
  1: { id: "0x11ace42c4c2ef0078b3f1ec5d9e8eab7a36795a6e402ad0c35d3bf3af25ea186", p: P("https://ethereum-sepolia-rpc.publicnode.com", 11155111), inbox: "0xdaC2C2d87e53c9de6900D7e90977059a0821A955", outbox: "0x583DE34F9C5e5c67a78D7F8551B2f5ae39658Dfc", dest: "0xAf80F8df996A1f9E09A87827A702FF8E0D0FeE72", name: "SEPOLIA", back: 60, done: { exec: false, ack: false } },
  2: { id: "0xaf4bf2ba0d73127cbc0a788d08b5ee38f75987bb371875b4895f7ea0ec4d2c9f", p: P("https://sepolia.base.org", 84532), inbox: "0x3D966043cEe7aa83EDaF816211b101bd041Ccd40", outbox: "0xa7f962Cd4A550Fc37801b4dF18456a8A9C506438", dest: "0xdD78a0eb836B699fC7F39C04EBe2379Ce74cB43C", name: "BASE", back: 400, done: { exec: false, ack: false } },
};
const inboxI = new ethers.Interface(A("write-ability/Inbox.sol/Inbox")), outboxAbi = A("write-ability/Outbox.sol/Outbox");
const t = () => new Date().toISOString().slice(11, 19); const until = Date.now() + 28 * 60 * 1000; const seen = new Set(); let n = 0;
for (const r of Object.values(R)) r.from = (await r.p.getBlockNumber()) - r.back;
while (Date.now() < until) {
  for (const [k, r] of Object.entries(R)) {
    try {
      const head = await r.p.getBlockNumber();
      for (const l of await r.p.getLogs({ address: r.inbox, fromBlock: r.from, toBlock: head })) { let e; try { e = inboxI.parseLog(l); } catch { continue; } if ((e.args.messageId ?? "").toLowerCase() !== r.id) continue; const key = `${k}:${e.name}`; if (seen.has(key)) continue; seen.add(key); const calls = e.name === "MessageExecuted" ? ` calls()=${await new ethers.Contract(r.dest, ["function calls() view returns (uint256)"], r.p).calls().catch(() => "?")}` : ""; console.log(`${t()} ${r.name} Inbox.${e.name} block ${l.blockNumber} tx ${l.transactionHash}${calls}`); if (e.name === "MessageExecuted") r.done.exec = true; }
      r.from = Math.max(r.from, head - r.back);
      if (!r.done.ack) { const acks = await new ethers.Contract(r.outbox, outboxAbi, cc).queryFilter("MessageAcknowledged", 69560, "latest"); const hit = acks.find((l) => (l.args?.messageId ?? "").toLowerCase() === r.id); if (hit) { r.done.ack = true; console.log(`${t()} ASC-DEVNET key ${k} MessageAcknowledged block ${hit.blockNumber} tx ${hit.transactionHash}`); } }
    } catch (e) { console.log(`${t()} key ${k} poll error: ${(e.shortMessage ?? e.message).slice(0, 90)}`); }
  }
  if (Object.values(R).every((r) => r.done.exec && r.done.ack)) { console.log("DONE: both routes delivered + acknowledged"); process.exit(0); }
  if (++n % 10 === 1) console.log(`${t()} polling… ${Object.entries(R).map(([k, r]) => `key${k}:exec=${r.done.exec} ack=${r.done.ack}`).join(" ")}`);
  await new Promise((r) => setTimeout(r, 30000));
}
console.log("WATCH WINDOW CLOSED"); process.exit(0);
