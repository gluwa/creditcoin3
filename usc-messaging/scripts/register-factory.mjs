// Register the deployed OutboxFactory for a chain_key in the CC runtime's SupportedChains, so the
// attestor's chain-info precompile (get_outbox_factory_address) resolves it.
// Sudo Substrate extrinsic (//Alice on --dev). Reads the factory address from /tmp/e2e-deploy.json.
//   node usc-messaging/scripts/register-factory.mjs
import { ApiPromise, WsProvider, Keyring } from "@polkadot/api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { readFileSync } from "node:fs";

const CHAIN_KEY = BigInt(process.env.DESTINATION_CHAIN_KEY ?? "2");
const WS = process.env.CREDITCOIN_SUBSTRATE_WS_URL || "ws://127.0.0.1:9944";
const factory = JSON.parse(readFileSync("/tmp/e2e-deploy.json", "utf8")).source.factory;
if (!factory) throw new Error("no source.factory in /tmp/e2e-deploy.json");

const api = await ApiPromise.create({ provider: new WsProvider(WS), noInitWarn: true });
await api.isReady;
await cryptoWaitReady();
const sudo = new Keyring({ type: "sr25519" }).addFromUri("//Alice");

console.log(`registering OutboxFactory ${factory} for chain_key ${CHAIN_KEY}…`);
await new Promise((resolve, reject) => {
  api.tx.sudo
    .sudo(api.tx.supportedChains.setOutboxFactoryAddr(CHAIN_KEY, factory))
    .signAndSend(sudo, ({ status, dispatchError }) => {
      if (dispatchError) reject(new Error(dispatchError.toString()));
      else if (status.isInBlock || status.isFinalized) resolve();
    });
});
console.log("✅ OutboxFactory registered");
await api.disconnect();
process.exit(0);
