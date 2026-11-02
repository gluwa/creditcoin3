import type { ApiPromise } from "@polkadot/api";
import type { KeyringPair } from "@polkadot/keyring/types";
import type { ethers } from "ethers";

export interface DevnetNetwork {
  name: string;
  evmChainId: number;
  evmRpc: string;
  substrateWs: string;
  deployFile: string;
  deprecated: boolean;
}
export const NETWORKS: Record<string, DevnetNetwork>;
export const DEFAULT_NETWORK: string;
export function network(): DevnetNetwork;
export function deployPath(net?: DevnetNetwork): string;
export function ccRpc(net?: DevnetNetwork, recordRpc?: string): string;
export function substrateWs(net?: DevnetNetwork): string;
export function ccProvider(
  net?: DevnetNetwork,
  recordRpc?: string,
): Promise<{ provider: ethers.JsonRpcProvider; rpc: string }>;
export function assertSudo(api: ApiPromise, pair: KeyringPair, net?: DevnetNetwork): Promise<string>;
