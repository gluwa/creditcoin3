// usc-dev: register a NEW destination chain for write-ability in the Creditcoin runtime (sudo).
//
// Step 1 of 3 when adding a destination chain to usc-devnet (chain id 42):
//   1. register-chain-devnet.mjs        (this)  — pallet side, assigns the chain key
//   2. deploy-dest-chain-devnet.mts     — destination EVM chain: AttestorRegistry, EOAValidator, Inbox
//   3. deploy-source-chain-devnet.mts   — Creditcoin: per-chain Outbox / vault / RelayerContractLite
//
// What it does, every step idempotent (storage is read first):
//   a. supportedChains.registerChain(chainId, name, …) unless chainIdAndNameToUniqKey(chainId, name)
//      already holds a key — then that key is reused. The live extrinsic shape is checked against
//      `api.tx.supportedChains.registerChain.meta.args` before submitting.
//   b. supportedChains.setOutboxFactoryAddr(chainKey, factory)     (factory = source.factory)
//   c. supportedChains.setWriteAbilityConfig(chainKey, bytes32(chainKey), true)
//   d. supportedChains.setCoreFee(chainKey, CORE_FEE_WEI)          (only when CORE_FEE_WEI is set)
//   e. supportedChains.setOutboxDiscoveryAddr(chainKey, source.outboxDiscovery) when the runtime
//      exposes it (#1292) and the deploy JSON has the address; otherwise skipped with a message.
// Finally the chain key is printed and written to usc-dev-deploy.json under
// chains.<chainKey>.{chainKey,destChainId,destChainName,registeredAt}; `source` / `dest` untouched.
//
// Env:
//   SUDO_URI                      seed/uri of the devnet sudo account            (required)
//   DEST_CHAIN_ID                 destination EVM chain id, e.g. 84532            (required)
//   DEST_CHAIN_NAME               e.g. "Base Sepolia"                             (required)
//   TARGET_SAMPLE_SIZE            attestors sampled per attestation               (required)
//   ATTESTATION_INTERVAL          blocks between attestations                     (required)
//   CHECKPOINT_INTERVAL           attestations per checkpoint                     (required)
//   MAX_ATTESTORS                 max attestors for the chain                     (required)
//   GENESIS_BLOCK                 first attested block, multiple of ATTESTATION_INTERVAL (required)
//   MATURITY                      e.g. "FixedDelay: 20", "EvmSafe", "EvmFinalized", "EvmLatest" (required)
//   CORE_FEE_WEI                  optional, attestcoin wei; unset = left as is
//   FACTORY_ADDR                  optional, default source.factory from the deploy JSON
//   CREDITCOIN_SUBSTRATE_WS_URL   default wss://rpc.usc-devnet.creditcoin.network
//   DEPLOY_OUT                    default ../usc-dev-deploy.json
//
// Keys used here are DEVNET-ONLY. Never point this at testnet/mainnet.
import { ApiPromise, WsProvider, Keyring } from "@polkadot/api";
import { cryptoWaitReady } from "@polkadot/util-crypto";
import { stringToU8a } from "@polkadot/util";
import { readFileSync, writeFileSync } from "node:fs";

const WS = process.env.CREDITCOIN_SUBSTRATE_WS_URL || "wss://rpc.usc-devnet.creditcoin.network";
const OUT = process.env.DEPLOY_OUT ?? new URL("../usc-dev-deploy.json", import.meta.url).pathname;

const need = (k) => {
  if (!process.env[k]) throw new Error(`missing ${k}`);
  return process.env[k];
};
const uint = (k) => {
  const v = need(k);
  if (!/^\d+$/.test(v)) throw new Error(`${k} must be a non-negative integer, got "${v}"`);
  return BigInt(v);
};

const SUDO_URI = need("SUDO_URI");
const DEST_CHAIN_ID = uint("DEST_CHAIN_ID");
const DEST_CHAIN_NAME = need("DEST_CHAIN_NAME");
const TARGET_SAMPLE_SIZE = uint("TARGET_SAMPLE_SIZE");
const ATTESTATION_INTERVAL = uint("ATTESTATION_INTERVAL");
const CHECKPOINT_INTERVAL = uint("CHECKPOINT_INTERVAL");
const MAX_ATTESTORS = uint("MAX_ATTESTORS");
const GENESIS_BLOCK = uint("GENESIS_BLOCK");
const MATURITY = need("MATURITY");
const CORE_FEE_WEI = process.env.CORE_FEE_WEI;

if (ATTESTATION_INTERVAL === 0n || CHECKPOINT_INTERVAL === 0n) {
  throw new Error("ATTESTATION_INTERVAL and CHECKPOINT_INTERVAL must be > 0 (zero bricks commit_attestation weights)");
}
if (GENESIS_BLOCK % ATTESTATION_INTERVAL !== 0n) {
  throw new Error(`GENESIS_BLOCK ${GENESIS_BLOCK} is not a multiple of ATTESTATION_INTERVAL ${ATTESTATION_INTERVAL}`);
}
// Same grammar as pallets/supported-chains is_valid_maturity_strategy.
if (!/^(EvmFinalized|EvmSafe|EvmLatest|FixedDelay:\s*\d+)$/.test(MATURITY)) {
  throw new Error(`MATURITY "${MATURITY}" is not one of EvmFinalized | EvmSafe | EvmLatest | "FixedDelay: N"`);
}
if (CORE_FEE_WEI !== undefined && !/^\d+$/.test(CORE_FEE_WEI)) throw new Error("CORE_FEE_WEI must be an integer (wei)");

const deploy = JSON.parse(readFileSync(OUT, "utf8"));
const source = deploy.source ?? {};
const factory = process.env.FACTORY_ADDR ?? source.factory;
if (!factory) throw new Error(`no source.factory in ${OUT} and FACTORY_ADDR unset`);
const discovery = source.outboxDiscovery;

const api = await ApiPromise.create({ provider: new WsProvider(WS), noInitWarn: true });
await api.isReady;
await cryptoWaitReady();
const sudo = new Keyring({ type: "sr25519" }).addFromUri(SUDO_URI);
console.log(`runtime ${api.runtimeVersion.specName.toString()}/${api.runtimeVersion.specVersion.toString()} at ${WS}`);
console.log("sudo account:", sudo.address);

const submit = (label, call) =>
  new Promise((resolve, reject) => {
    api.tx.sudo.sudo(call).signAndSend(sudo, ({ status, dispatchError, events }) => {
      if (dispatchError) return reject(new Error(`${label}: ${dispatchError.toString()}`));
      if (status.isInBlock) {
        const failed = events.find((e) => api.events.sudo.Sudid.is(e.event) && e.event.data[0].isErr);
        if (failed) return reject(new Error(`${label}: inner call failed: ${failed.event.data[0].asErr.toString()}`));
        console.log(`✅ ${label} in block ${status.asInBlock.toHex()}`);
        resolve(events);
      }
    });
  });

// Key 2 of ChainIdAndNameToUniqKey is Vec<u8> of the UTF-8 name; pass bytes so polkadot-js never
// tries to interpret the name as hex.
const nameBytes = stringToU8a(DEST_CHAIN_NAME);
const lookupChainKey = async () => {
  const k = await api.query.supportedChains.chainIdAndNameToUniqKey(DEST_CHAIN_ID, nameBytes);
  return k.isSome ? BigInt(k.unwrap().toString()) : null;
};

// (a) register_chain — or reuse.
let chainKey = await lookupChainKey();
if (chainKey !== null) {
  console.log(`ℹ️  (${DEST_CHAIN_ID}, "${DEST_CHAIN_NAME}") is already registered as chain key ${chainKey} — reusing it`);
} else {
  const tx = api.tx.supportedChains?.registerChain;
  if (!tx) throw new Error("runtime has no supportedChains.registerChain");
  // Guard against a runtime whose register_chain shape differs from what we encode below:
  // exact argument names in this order, and Option-ness per argument (type names carry
  // runtime-specific path prefixes, so they are only checked for the Option<> wrapper).
  const EXPECTED = [
    ["chainId", false], ["chainName", false], ["targetSampleSize", true],
    ["chainAttestationInterval", true], ["attestationCheckpointInterval", true],
    ["maxAttestors", true], ["maxInvulnerables", true],
    ["attestationChainGenesisBlockNumber", true], ["encoding", false], ["maturityStrategy", true],
  ];
  const live = tx.meta.args.map((a) => [a.name.toString(), a.type.toString()]);
  const shapeOk = live.length === EXPECTED.length &&
    live.every(([n, t], i) => n === EXPECTED[i][0] && t.startsWith("Option<") === EXPECTED[i][1]);
  if (!shapeOk) {
    throw new Error(
      `supportedChains.registerChain on this runtime has args\n  ${live.map((a) => a.join(": ")).join("\n  ")}\n` +
      `but this script encodes\n  ${EXPECTED.map(([n, o]) => `${n}: ${o ? "Option<…>" : "…"}`).join("\n  ")}\nRefusing to submit; update the script.`,
    );
  }
  await submit(
    `registerChain(${DEST_CHAIN_ID}, "${DEST_CHAIN_NAME}", target=${TARGET_SAMPLE_SIZE}, interval=${ATTESTATION_INTERVAL}, checkpoint=${CHECKPOINT_INTERVAL}, maxAttestors=${MAX_ATTESTORS}, genesis=${GENESIS_BLOCK}, V1, "${MATURITY}")`,
    tx(
      DEST_CHAIN_ID, DEST_CHAIN_NAME,
      TARGET_SAMPLE_SIZE, ATTESTATION_INTERVAL, CHECKPOINT_INTERVAL, MAX_ATTESTORS,
      null /* max_invulnerables: None */, GENESIS_BLOCK, "V1", MATURITY,
    ),
  );
  chainKey = await lookupChainKey();
  if (chainKey === null) throw new Error("registerChain landed but chainIdAndNameToUniqKey has no entry for it");
  console.log(`   assigned chain key ${chainKey}`);
}

const supported = await api.query.supportedChains.supportedChains(chainKey);
if (!supported.isSome) throw new Error(`supportedChains(${chainKey}) is empty — registration did not stick`);
console.log(`   supportedChains(${chainKey}) = ${JSON.stringify(supported.unwrap().toHuman())}`);

// (b) OutboxFactory — the chain-info precompile's get_outbox_factory_address(chainKey).
const curFactory = await api.query.supportedChains.outboxFactories(chainKey);
if (curFactory.isSome && curFactory.unwrap().toString().toLowerCase() === factory.toLowerCase()) {
  console.log(`✅ outboxFactories(${chainKey}) already ${factory}`);
} else {
  await submit(`setOutboxFactoryAddr(${chainKey}, ${factory})`, api.tx.supportedChains.setOutboxFactoryAddr(chainKey, factory));
}

// (c) WriteAbilityConfig — bytes32 chain key with the value in the low 8 bytes (chain_key_to_bytes32).
const chainKeyBytes32 = "0x" + chainKey.toString(16).padStart(64, "0");
const curWa = await api.query.supportedChains.writeAbilityConfigs(chainKey);
const waOk = curWa.isSome &&
  curWa.unwrap().writeAbilityChainKey.toHex().toLowerCase() === chainKeyBytes32 &&
  curWa.unwrap().messageAttestationEnabled.isTrue;
if (waOk) {
  console.log(`✅ writeAbilityConfigs(${chainKey}) already {${chainKeyBytes32}, enabled}`);
} else {
  await submit(`setWriteAbilityConfig(${chainKey}, ${chainKeyBytes32}, true)`,
    api.tx.supportedChains.setWriteAbilityConfig(chainKey, chainKeyBytes32, true));
}

// (d) core fee (optional).
if (CORE_FEE_WEI !== undefined) {
  const cur = await api.query.supportedChains.coreFees(chainKey);
  if (cur.isSome && BigInt(cur.unwrap().amount.toString()) === BigInt(CORE_FEE_WEI)) {
    console.log(`✅ coreFees(${chainKey}) already ${CORE_FEE_WEI}`);
  } else {
    await submit(`setCoreFee(${chainKey}, ${CORE_FEE_WEI})`, api.tx.supportedChains.setCoreFee(chainKey, CORE_FEE_WEI));
  }
} else {
  console.log(`(core fee left unset — get_core_fee(${chainKey}) stays as is; set later with CORE_FEE_WEI)`);
}

// (e) OutboxDiscovery registry address (#1292 runtime only).
const setDiscovery = api.tx.supportedChains?.setOutboxDiscoveryAddr;
if (!setDiscovery) {
  console.log(`⏭️  runtime ${api.runtimeVersion.specVersion.toString()} has no supportedChains.setOutboxDiscoveryAddr (pre-#1292) — skipped; rerun after the upgrade`);
} else if (!discovery) {
  console.log("⏭️  no source.outboxDiscovery in the deploy JSON — skipping setOutboxDiscoveryAddr");
} else {
  const cur = await api.query.supportedChains.outboxDiscoveries(chainKey);
  if (cur.isSome && cur.unwrap().toString().toLowerCase() === discovery.toLowerCase()) {
    console.log(`✅ outboxDiscoveries(${chainKey}) already ${discovery}`);
  } else {
    await submit(`setOutboxDiscoveryAddr(${chainKey}, ${discovery})`, setDiscovery(chainKey, discovery));
  }
}

// Record. Only chains.<chainKey>; never touch source / dest.
deploy.chains ??= {};
const prev = deploy.chains[String(chainKey)] ?? {};
deploy.chains[String(chainKey)] = {
  ...prev,
  chainKey: Number(chainKey),
  destChainId: Number(DEST_CHAIN_ID),
  destChainName: DEST_CHAIN_NAME,
  registeredAt: prev.registeredAt ?? new Date().toISOString(),
};
writeFileSync(OUT, JSON.stringify(deploy, null, 2) + "\n");

console.log("");
console.log("==================================================");
console.log(`  CHAIN KEY for ${DEST_CHAIN_NAME} (${DEST_CHAIN_ID}): ${chainKey}`);
console.log("==================================================");
console.log(`recorded chains.${chainKey} in ${OUT}`);
console.log(`next: CHAIN_KEY=${chainKey} DEST_CHAIN_ID=${DEST_CHAIN_ID} npx tsx scripts/deploy-dest-chain-devnet.mts`);
await api.disconnect();
process.exit(0);
