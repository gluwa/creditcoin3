// Devnet network selection for the write-ability deploy / ops scripts.
//
// Every *-devnet script used to hard-code usc-devnet (Creditcoin EVM chain id 42, the
// rpc.usc-devnet host and usc-dev-deploy.json). Since 2026-10-01 everything deploys on
// asc-devnet (chain id 102037) and usc-devnet is frozen, so the network is now a table entry
// selected with NETWORK=<name> (default asc-devnet). Only devnets are listed on purpose: these
// scripts carry devnet-only keys and must never be pointed at testnet or mainnet.
//
// Per-network overrides keep working: CC_RPC (EVM JSON-RPC), CREDITCOIN_SUBSTRATE_WS_URL
// (Substrate WS) and DEPLOY_OUT (deploy record) take precedence over the table.
import { ethers } from "ethers";

export const NETWORKS = {
  "asc-devnet": {
    name: "asc-devnet",
    evmChainId: 102037,
    evmRpc: "https://rpc.asc-devnet.creditcoin.network",
    substrateWs: "wss://rpc.asc-devnet.creditcoin.network",
    deployFile: "asc-devnet-deploy.json",
    deprecated: false,
  },
  "usc-devnet": {
    name: "usc-devnet",
    evmChainId: 42,
    evmRpc: "https://rpc.usc-devnet.creditcoin.network",
    substrateWs: "wss://rpc.usc-devnet.creditcoin.network",
    deployFile: "usc-dev-deploy.json",
    // Deprecated 2026-10-01: no rolls, cutovers or contract changes there any more. Selectable
    // only to read its record or claim/inspect what is still deployed.
    deprecated: true,
  },
};

export const DEFAULT_NETWORK = "asc-devnet";

/** Resolve the selected network from NETWORK (default asc-devnet). Throws on unknown names. */
export function network() {
  const name = process.env.NETWORK ?? DEFAULT_NETWORK;
  const net = NETWORKS[name];
  if (!net) {
    throw new Error(`unknown NETWORK="${name}"; known devnets: ${Object.keys(NETWORKS).join(", ")}`);
  }
  if (net.deprecated) {
    console.warn(`⚠️  NETWORK=${name} is deprecated (frozen since 2026-10-01); nothing new should be deployed there`);
  }
  return net;
}

/** Path of the deploy record: DEPLOY_OUT, else <usc-messaging>/<deployFile>. */
export function deployPath(net = network()) {
  return process.env.DEPLOY_OUT ?? new URL(`../${net.deployFile}`, import.meta.url).pathname;
}

/** Creditcoin EVM JSON-RPC URL: CC_RPC, else the record's source.rpc, else the table. */
export function ccRpc(net = network(), recordRpc) {
  return process.env.CC_RPC ?? recordRpc ?? net.evmRpc;
}

/** Creditcoin Substrate WS URL: CREDITCOIN_SUBSTRATE_WS_URL, else the table. */
export function substrateWs(net = network()) {
  return process.env.CREDITCOIN_SUBSTRATE_WS_URL || net.substrateWs;
}

/**
 * ethers provider for the Creditcoin EVM pinned to the network's chain id. `staticNetwork`
 * skips ethers' own detection, so the live chain id is checked once here instead: a wrong
 * CC_RPC fails before any transaction is signed.
 */
export async function ccProvider(net = network(), recordRpc) {
  const rpc = ccRpc(net, recordRpc);
  const provider = new ethers.JsonRpcProvider(rpc, net.evmChainId, { staticNetwork: true, polling: true });
  provider.pollingInterval = 1000;
  const live = Number((await provider.getNetwork()).chainId);
  const actual = Number(await provider.send("eth_chainId", []));
  if (actual !== net.evmChainId || live !== net.evmChainId) {
    throw new Error(`${rpc} reports EVM chain id ${actual}, but NETWORK=${net.name} expects ${net.evmChainId}`);
  }
  return { provider, rpc };
}

/**
 * Sudo guard for the @polkadot/api scripts: the key derived from SUDO_URI must be the chain's
 * current sudo.key(), so a key for another network (or a typo) fails before anything is sent.
 */
export async function assertSudo(api, pair, net = network()) {
  const onChain = (await api.query.sudo.key()).toString();
  if (onChain !== pair.address) {
    throw new Error(
      `SUDO_URI derives ${pair.address}, but sudo.key() on ${net.name} is ${onChain} — wrong key or wrong NETWORK`,
    );
  }
  return onChain;
}
