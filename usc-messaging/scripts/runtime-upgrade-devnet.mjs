// usc-dev runtime upgrade (sudo): sudo.sudoUncheckedWeight(system.setCode(wasm)) with pre-flight checks.
//
//   WASM=/path/to/creditcoin3_runtime.compact.compressed.wasm   # the `subwasm` artifact of the
//                                                                 # "Build WASM Runtime" run for the commit
//   EXPECT_SPEC_VERSION=136 SUDO_URI='<usc-dev sudo seed>' node scripts/runtime-upgrade-devnet.mjs [--dry-run]
//
// Pre-flight: the chain is usc-devnet; the sudo key matches SUDO_URI; the wasm is not already the
// on-chain :code; EXPECT_SPEC_VERSION (from `subwasm info <wasm>`, run it yourself first) is strictly
// higher than the live spec_version. Then submits the unchecked-weight sudo call, waits for inclusion
// and `system.CodeUpdated`, and polls until the live spec_version changes.
//
// Before running against usc-devnet, run try-runtime against live state (the branch's try-runtime CI is
// red and undiagnosed): the migrations between the live runtime and this wasm must be exercised.
// Devnet only. Never point this at testnet or mainnet.
import { ApiPromise, WsProvider, Keyring } from "@polkadot/api";
import { cryptoWaitReady, blake2AsHex } from "@polkadot/util-crypto";
import { u8aToHex } from "@polkadot/util";
import { readFileSync } from "node:fs";

const WS = process.env.CREDITCOIN_SUBSTRATE_WS_URL || "wss://rpc.usc-devnet.creditcoin.network";
const DRY = process.argv.includes("--dry-run");
if (!process.env.WASM) throw new Error("need WASM=<path to the runtime .wasm>");
if (!process.env.EXPECT_SPEC_VERSION) throw new Error("need EXPECT_SPEC_VERSION=<spec_version embedded in the wasm, from `subwasm info`>");
if (!process.env.SUDO_URI && !DRY) throw new Error("need SUDO_URI (or --dry-run)");
if (!/usc-devnet/.test(WS)) throw new Error(`refusing: ${WS} is not the usc-devnet RPC`);
const expectSpec = Number(process.env.EXPECT_SPEC_VERSION);

const wasm = readFileSync(process.env.WASM);
const api = await ApiPromise.create({ provider: new WsProvider(WS), noInitWarn: true });
await api.isReady;
await cryptoWaitReady();

const live = api.runtimeVersion;
const liveSpec = live.specVersion.toNumber();
console.log(`live:  ${live.specName} spec ${liveSpec} impl ${live.implVersion} tx ${live.transactionVersion}`);
if (!(expectSpec > liveSpec)) throw new Error(`EXPECT_SPEC_VERSION ${expectSpec} is not higher than the live spec_version ${liveSpec}`);
console.log(`wasm:  ${wasm.length} bytes, blake2-256 ${blake2AsHex(wasm)}, expected spec ${expectSpec}`);

const currentCode = (await api.rpc.state.getStorage(":code")).toU8a(true);
if (blake2AsHex(currentCode) === blake2AsHex(wasm)) throw new Error("this wasm is already the on-chain runtime");
console.log(`chain :code is ${currentCode.length} bytes, blake2-256 ${blake2AsHex(currentCode)}`);

const sudoKey = (await api.query.sudo.key()).toString();
let sudo = null;
if (process.env.SUDO_URI) {
  sudo = new Keyring({ type: "sr25519" }).addFromUri(process.env.SUDO_URI);
  if (sudo.address !== sudoKey) throw new Error(`SUDO_URI resolves to ${sudo.address} but the chain's sudo key is ${sudoKey}`);
  console.log("sudo:", sudo.address);
} else {
  console.log("sudo key on chain:", sudoKey);
}

const call = api.tx.system.setCode(u8aToHex(wasm));
const tx = api.tx.sudo.sudoUncheckedWeight(call, { refTime: 0, proofSize: 0 });
console.log(`call: sudo.sudoUncheckedWeight(system.setCode) ${tx.encodedLength} bytes`);
if (DRY) {
  console.log("dry run: not submitting");
  await api.disconnect();
  process.exit(0);
}

await new Promise((resolve, reject) => {
  tx.signAndSend(sudo, ({ status, dispatchError, events }) => {
    if (dispatchError) return reject(new Error(dispatchError.toString()));
    if (status.isInBlock) {
      const failed = events.find((e) => api.events.sudo.Sudid.is(e.event) && e.event.data[0].isErr);
      if (failed) return reject(new Error(`inner setCode failed: ${failed.event.data[0].asErr.toString()}`));
      const updated = events.some((e) => api.events.system.CodeUpdated.is(e.event));
      console.log(`✅ setCode in block ${status.asInBlock.toHex()}${updated ? " (system.CodeUpdated)" : ""}`);
      resolve();
    }
  }).catch(reject);
});

for (let i = 0; i < 30; i++) {
  await new Promise((r) => setTimeout(r, 6000));
  const v = await api.rpc.state.getRuntimeVersion();
  if (v.specVersion.toNumber() !== liveSpec) {
    console.log(`🎉 runtime is now ${v.specName} spec ${v.specVersion} (was ${liveSpec})`);
    await api.disconnect();
    process.exit(0);
  }
}
console.error("spec_version did not change within 3 minutes; check the node logs");
await api.disconnect();
process.exit(1);
