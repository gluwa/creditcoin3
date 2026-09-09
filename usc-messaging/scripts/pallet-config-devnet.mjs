// usc-dev write-ability pallet config (sudo): register the new OutboxFactory, set the
// WriteAbilityConfig, (optionally) the core fee, and — once the #1292 runtime is live — the
// OutboxDiscovery registry address for chain_key 8.
// Env: SUDO_URI (seed/uri of the devnet sudo account), optional CORE_FEE_WEI (default: skip),
//      optional ONLY_DISCOVERY=true to run just the set_outbox_discovery_addr step.
// Reads the factory / discovery addresses from usc-dev-deploy.json (written by
// deploy-source-devnet / deploy-discovery-devnet).
import { ApiPromise, WsProvider, Keyring } from "@polkadot/api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { readFileSync } from "node:fs";

const CHAIN_KEY = 8n;
const WS = process.env.CREDITCOIN_SUBSTRATE_WS_URL || "wss://rpc.usc-devnet.creditcoin.network";
const OUT = process.env.DEPLOY_OUT ?? "/tmp/usc-dev-deploy.json";
const source = JSON.parse(readFileSync(OUT, "utf8")).source ?? {};
const factory = source.factory;
const discovery = process.env.DISCOVERY_ADDR ?? source.outboxDiscovery;
const ONLY_DISCOVERY = process.env.ONLY_DISCOVERY === "true";
if (!factory && !ONLY_DISCOVERY) throw new Error(`no source.factory in ${OUT}`);
if (!process.env.SUDO_URI) throw new Error("need SUDO_URI");

// bytes32 chain key: value in the low 8 bytes (matches chain_key_to_bytes32 in Rust).
const chainKeyBytes32 = "0x" + CHAIN_KEY.toString(16).padStart(64, "0");

const api = await ApiPromise.create({ provider: new WsProvider(WS), noInitWarn: true });
await api.isReady;
await cryptoWaitReady();
const sudo = new Keyring({ type: "sr25519" }).addFromUri(process.env.SUDO_URI);
console.log("sudo account:", sudo.address);

const submit = (label, call) =>
  new Promise((resolve, reject) => {
    api.tx.sudo.sudo(call).signAndSend(sudo, ({ status, dispatchError, events }) => {
      if (dispatchError) return reject(new Error(`${label}: ${dispatchError.toString()}`));
      if (status.isInBlock) {
        const failed = events.find((e) => api.events.sudo.Sudid.is(e.event) && e.event.data[0].isErr);
        if (failed) return reject(new Error(`${label}: inner call failed: ${failed.event.data[0].asErr.toString()}`));
        console.log(`✅ ${label} in block ${status.asInBlock.toHex()}`);
        resolve();
      }
    });
  });

// Discovery registry (creditcoin3 #1292): attestors and relayer resolve the Outbox through
// chain-info.get_outbox_discovery_address(chainKey) -> OutboxDiscovery.defaultOutbox(chainKey).
// The extrinsic only exists on a runtime that includes #1292; on an older runtime this is skipped
// with a clear message instead of failing the whole script.
async function setDiscovery() {
  if (!discovery) { console.log("ℹ️  no source.outboxDiscovery / DISCOVERY_ADDR — skipping set_outbox_discovery_addr"); return; }
  const call = api.tx.supportedChains?.setOutboxDiscoveryAddr;
  if (!call) { console.log(`⏭️  runtime ${api.runtimeVersion.specVersion.toString()} has no supportedChains.setOutboxDiscoveryAddr yet (needs #1292) — skipping; rerun with ONLY_DISCOVERY=true after the upgrade`); return; }
  const current = await api.query.supportedChains.outboxDiscoveries(CHAIN_KEY);
  if (current.isSome && current.unwrap().toString().toLowerCase() === discovery.toLowerCase()) {
    console.log(`✅ outboxDiscoveries(${CHAIN_KEY}) already ${discovery}`); return;
  }
  await submit(`setOutboxDiscoveryAddr(${CHAIN_KEY}, ${discovery})`, call(CHAIN_KEY, discovery));
  const after = await api.query.supportedChains.outboxDiscoveries(CHAIN_KEY);
  if (!after.isSome || after.unwrap().toString().toLowerCase() !== discovery.toLowerCase()) {
    throw new Error(`set_outbox_discovery_addr landed but storage reads ${after.toString()}`);
  }
}
if (ONLY_DISCOVERY) { await setDiscovery(); await api.disconnect(); process.exit(0); }

await submit(`setOutboxFactoryAddr(${CHAIN_KEY}, ${factory})`,
  api.tx.supportedChains.setOutboxFactoryAddr(CHAIN_KEY, factory));

await submit(`setWriteAbilityConfig(${CHAIN_KEY}, ${chainKeyBytes32}, true)`,
  api.tx.supportedChains.setWriteAbilityConfig(CHAIN_KEY, chainKeyBytes32, true));

if (process.env.CORE_FEE_WEI) {
  await submit(`setCoreFee(${CHAIN_KEY}, ${process.env.CORE_FEE_WEI})`,
    api.tx.supportedChains.setCoreFee(CHAIN_KEY, process.env.CORE_FEE_WEI));
} else {
  console.log("(core fee left unset — get_core_fee(8) stays 0; set later with CORE_FEE_WEI)");
}

await setDiscovery();

await api.disconnect();
process.exit(0);
