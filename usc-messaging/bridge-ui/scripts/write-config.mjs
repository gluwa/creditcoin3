// Populate public/bridge-config.json with bridge addresses from env (the deploy emits these).
//   CC_TOK CC_BRIDGE AN_TOK AN_BRIDGE  (and optional CC_CHAIN_ID / DEST_CHAIN_ID)
//   node scripts/write-config.mjs
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const cfgPath = join(here, "..", "public", "bridge-config.json");
const cfg = JSON.parse(readFileSync(cfgPath, "utf8"));
const e = process.env;

cfg.chains.creditcoin.token = e.CC_TOK || cfg.chains.creditcoin.token;
cfg.chains.creditcoin.bridge = e.CC_BRIDGE || cfg.chains.creditcoin.bridge;
cfg.chains.dest.token = e.AN_TOK || cfg.chains.dest.token;
cfg.chains.dest.bridge = e.AN_BRIDGE || cfg.chains.dest.bridge;
if (e.CC_CHAIN_ID) cfg.chains.creditcoin.chainId = Number(e.CC_CHAIN_ID);
if (e.DEST_CHAIN_ID) cfg.chains.dest.chainId = Number(e.DEST_CHAIN_ID);

writeFileSync(cfgPath, JSON.stringify(cfg, null, 2) + "\n");
console.log("✅ wrote", cfgPath);
console.log(JSON.stringify(cfg.chains, null, 2));
