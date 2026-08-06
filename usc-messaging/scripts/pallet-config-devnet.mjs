// usc-dev write-ability pallet config (sudo): register the new OutboxFactory, set the
// WriteAbilityConfig, and (optionally) the core fee for chain_key 8.
// Env: SUDO_URI (seed/uri of the devnet sudo account), optional CORE_FEE_WEI (default: skip).
// Reads the factory address from /tmp/usc-dev-deploy.json (written by deploy-source-devnet).
import { ApiPromise, WsProvider, Keyring } from "@polkadot/api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { readFileSync } from "node:fs";

const CHAIN_KEY = 8n;
const WS = process.env.CREDITCOIN_SUBSTRATE_WS_URL || "wss://rpc.usc-devnet.creditcoin.network";
const OUT = process.env.DEPLOY_OUT ?? "/tmp/usc-dev-deploy.json";
const factory = JSON.parse(readFileSync(OUT, "utf8")).source?.factory;
if (!factory) throw new Error(`no source.factory in ${OUT}`);
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

await api.disconnect();
process.exit(0);
