/**
 * USC (Creditcoin3) chain client
 *
 * Queries storage for supported chains, attestations, checkpoints.
 */

import { ApiPromise, WsProvider } from "@polkadot/api";
import { withTimeout } from "./timeout.ts";

const USC_TIMEOUT_MS = 30_000;

export interface SupportedChain {
  chainId: number;
  chainName: string;
  chainKey: number;
  maturityStrategy: string;
}

export const DEFAULT_MATURITY_STRATEGY = "FixedDelay: 10";
const DEFAULT_MATURITY_DELAY = 10;

/**
 * Maps a maturity strategy string (as stored on-chain) to its block delay.
 * Mirrors `MaturityStrategy::maturity_delay` in primitives/supported-chains.
 * Unknown strategies fall back to the default with a warning.
 */
export function getMaturityDelay(strategy: string): number {
  switch (strategy) {
    case "EvmFinalized":
      return 64;
    case "EvmSafe":
      return 32;
    case "EvmLatest":
      return 0;
    default: {
      const m = strategy.match(/^FixedDelay:\s*(\d+)$/);
      if (m) return Number(m[1]);
      console.warn(
        `[usc] unknown maturity strategy "${strategy}", falling back to ${DEFAULT_MATURITY_STRATEGY}`,
      );
      return DEFAULT_MATURITY_DELAY;
    }
  }
}

export interface LastCheckpoint {
  blockNumber: number;
  digest: string;
}

export interface SignedAttestation {
  headerNumber: number;
  headerHash: string;
  root: string;
  prevDigest?: string;
  digest: string;
}

let api: ApiPromise | null = null;
let verboseLogging = false;

/** Enable verbose error logging for USC queries. */
export function setVerbose(v: boolean): void {
  verboseLogging = v;
}

export async function connect(wsUrl: string): Promise<void> {
  const provider = new WsProvider(wsUrl);
  await withTimeout(
    (async () => {
      api = await ApiPromise.create({ provider, noInitWarn: true });
      await api.isReady;
    })(),
    USC_TIMEOUT_MS,
    "USC WebSocket connection timeout",
  );
}

export async function getNativeBalance(address: string): Promise<bigint> {
  return await withTimeout(getNativeBalanceImpl(address), USC_TIMEOUT_MS);
}

async function getNativeBalanceImpl(address: string): Promise<bigint> {
  const a = getApi();
  const accountInfo = await a.query.system.account(address) as unknown as {
    data?: {
      free?: { toString: () => string };
    };
  };
  const free = accountInfo.data?.free?.toString();
  if (!free || !/^\d+$/.test(free)) {
    throw new Error(`unexpected native balance response for ${address}`);
  }
  return BigInt(free);
}

export function disconnect(): void {
  if (api) {
    api.disconnect();
    api = null;
  }
}

function getApi(): ApiPromise {
  if (!api) throw new Error("USC API not connected");
  return api;
}

function findPallet(api: ApiPromise, substring: string): string | undefined {
  return Object.keys(api.query).find((p) =>
    p.toLowerCase().includes(substring.toLowerCase())
  );
}

function findStorageKey(
  storage: Record<string, unknown>,
  ...substrings: string[]
): string | undefined {
  return Object.keys(storage).find((s) => {
    const lower = s.toLowerCase();
    return substrings.every((sub) => lower.includes(sub.toLowerCase()));
  });
}

/**
 * Get supported chains from storage.
 * Tries common pallet names (metadata may vary).
 */
export async function getSupportedChains(): Promise<SupportedChain[]> {
  return await withTimeout(getSupportedChainsImpl(), USC_TIMEOUT_MS);
}

async function getSupportedChainsImpl(): Promise<SupportedChain[]> {
  const a = getApi();
  const chains: SupportedChain[] = [];

  try {
    const attestationPallet = findPallet(a, "attestation");
    const supportedPallet = findPallet(a, "supported");

    if (supportedPallet) {
      const storage = (a.query as Record<string, Record<string, unknown>>)[
        supportedPallet
      ] as Record<string, unknown>;
      const storageName = storage
        ? findStorageKey(storage, "chain")
        : undefined;
      if (storageName && storage) {
        const entries = await (storage[storageName] as {
          entries: () => Promise<[unknown, unknown][]>;
        }).entries();
        for (const [key, value] of entries) {
          try {
            const decodedKey = key as { toHuman?: () => unknown };
            const keyHuman = decodedKey?.toHuman?.() as
              | Array<string>
              | undefined;
            const chainKey = keyHuman && keyHuman.length > 0
              ? Number(keyHuman[0].replaceAll(",", ""))
              : null;

            const decoded = value as { toHuman?: () => unknown };
            const human = decoded?.toHuman?.() as
              | Record<string, unknown>
              | undefined;
            if (human) {
              const chainIdStr =
                (human.chainId ?? human.chain_id ?? "") as string;
              const chainId = Number(chainIdStr.replaceAll(",", "")) || 0;
              const nameBytes = human.chainName ?? human.chain_name;
              const chainName = Array.isArray(nameBytes)
                ? String.fromCharCode(...(nameBytes as number[]))
                : String(nameBytes ?? "");
              const maturityStrategy = String(
                human.maturityStrategy ?? human.maturity_strategy ??
                  DEFAULT_MATURITY_STRATEGY,
              );
              if (chainId && chainName && chainKey) {
                chains.push({
                  chainId,
                  chainName,
                  chainKey,
                  maturityStrategy,
                });
              }
            }
          } catch (e) {
            if (verboseLogging) console.warn("[usc] error:", e);
            // Skip malformed entries
          }
        }
      }
    }

    // Fallback: use attestation.LastDigest keys to discover chain_keys
    if (chains.length === 0 && attestationPallet) {
      const attestationQuery =
        (a.query as Record<string, Record<string, unknown>>)[attestationPallet];
      const lastDigest = attestationQuery?.lastDigest ??
        attestationQuery?.LastDigest;
      if (
        lastDigest &&
        typeof (lastDigest as { keys: () => Promise<unknown[]> }).keys ===
          "function"
      ) {
        const keys =
          await (lastDigest as { keys: () => Promise<{ args: unknown[] }[]> })
            .keys();
        for (const k of keys) {
          const chainKey = Array.isArray(k.args)
            ? Number(k.args[0])
            : Number(k);
          if (chainKey && !chains.some((c) => c.chainKey === chainKey)) {
            chains.push({
              chainId: 0,
              chainName: `Chain ${chainKey}`,
              chainKey,
              maturityStrategy: DEFAULT_MATURITY_STRATEGY,
            });
          }
        }
      }
    }
  } catch (e) {
    console.warn("getSupportedChains error:", e);
  }

  return chains;
}

export async function getLastDigest(chainKey: number): Promise<string | null> {
  return await withTimeout(getLastDigestImpl(chainKey), USC_TIMEOUT_MS);
}

async function getLastDigestImpl(chainKey: number): Promise<string | null> {
  const a = getApi();
  try {
    const p = findPallet(a, "attestation");
    if (!p) return null;
    const storage = (a.query as Record<string, Record<string, unknown>>)[
      p
    ] as Record<string, unknown>;
    const lastDigest = storage?.lastDigest ?? storage?.LastDigest;
    if (!lastDigest || typeof lastDigest !== "function") return null;
    const result = await (lastDigest as (key: number) => Promise<unknown>)(
      chainKey,
    );
    if (!result) return null;
    const hex = (result as { toHex?: () => string }).toHex?.() ??
      (result as { toString: () => string }).toString?.();
    return hex ?? null;
  } catch (e) {
    if (verboseLogging) console.warn("[usc] error:", e);
    return null;
  }
}

export async function getLastCheckpoint(
  chainKey: number,
): Promise<LastCheckpoint | null> {
  return await withTimeout(getLastCheckpointImpl(chainKey), USC_TIMEOUT_MS);
}

async function getLastCheckpointImpl(
  chainKey: number,
): Promise<LastCheckpoint | null> {
  const a = getApi();
  try {
    const p = findPallet(a, "attestation");
    if (!p) return null;
    const storage = (a.query as Record<string, Record<string, unknown>>)[
      p
    ] as Record<string, unknown>;
    const lastCp = storage?.lastCheckpoint ?? storage?.LastCheckpoint;
    if (!lastCp || typeof lastCp !== "function") return null;
    const result = await (lastCp as (key: number) => Promise<unknown>)(
      chainKey,
    );
    if (!result) return null;
    const human = (result as { toHuman?: () => unknown }).toHuman?.() as
      | Record<string, unknown>
      | undefined;
    if (!human) return null;
    const blockNumber = parseBlockNumber(
      human.blockNumber ?? human.block_number ?? 0,
    );
    const digest = String(human.digest ?? "").replace(/^0x/, "");
    return { blockNumber, digest };
  } catch (e) {
    if (verboseLogging) console.warn("[usc] error:", e);
    return null;
  }
}

export async function getCheckpointInterval(chainKey: number): Promise<number> {
  return await withTimeout(getCheckpointIntervalImpl(chainKey), USC_TIMEOUT_MS);
}

async function getCheckpointIntervalImpl(chainKey: number): Promise<number> {
  const a = getApi();
  try {
    const p = findPallet(a, "attestation");
    if (!p) return 180;
    const storage = (a.query as Record<string, Record<string, unknown>>)[
      p
    ] as Record<string, unknown>;
    const interval = storage?.attestationCheckpointInterval ??
      storage?.AttestationCheckpointInterval;
    if (!interval || typeof interval !== "function") return 180;
    const result = await (interval as (key: number) => Promise<unknown>)(
      chainKey,
    );
    return parseBlockNumber(result ?? 180) || 180;
  } catch (e) {
    if (verboseLogging) console.warn("[usc] error:", e);
    return 180;
  }
}

export async function getAttestationInterval(
  chainKey: number,
): Promise<number> {
  return await withTimeout(
    getAttestationIntervalImpl(chainKey),
    USC_TIMEOUT_MS,
  );
}

async function getAttestationIntervalImpl(chainKey: number): Promise<number> {
  const a = getApi();
  try {
    const p = findPallet(a, "attestation");
    if (!p) return 10;
    const storage = (a.query as Record<string, Record<string, unknown>>)[
      p
    ] as Record<string, unknown>;
    const interval = storage?.chainAttestationInterval ??
      storage?.ChainAttestationInterval;
    if (!interval || typeof interval !== "function") return 10;
    const result = await (interval as (key: number) => Promise<unknown>)(
      chainKey,
    );
    return parseBlockNumber(result ?? 10) || 10;
  } catch (e) {
    if (verboseLogging) console.warn("[usc] error:", e);
    return 10;
  }
}

/** Parse block number from Polkadot value (toHuman uses commas; raw has toNumber) */
function parseBlockNumber(v: unknown): number {
  if (typeof v === "number" && !isNaN(v)) return v;
  if (typeof v === "string") {
    const n = Number(v.replace(/,/g, ""));
    return isNaN(n) ? 0 : n;
  }
  const obj = v as { toNumber?: () => number; toString?: () => string };
  if (typeof obj?.toNumber === "function") return obj.toNumber();
  if (typeof obj?.toString === "function") {
    return parseBlockNumber(obj.toString());
  }
  return 0;
}

/**
 * Get attestation by chain_key and digest.
 * Returns header_number and header_hash from the attestation.
 */
export async function getAttestationByDigest(
  chainKey: number,
  digestHex: string,
): Promise<SignedAttestation | null> {
  return await withTimeout(
    getAttestationByDigestImpl(chainKey, digestHex),
    USC_TIMEOUT_MS,
  );
}

async function getAttestationByDigestImpl(
  chainKey: number,
  digestHex: string,
): Promise<SignedAttestation | null> {
  const a = getApi();
  try {
    const p = findPallet(a, "attestation");
    if (!p) return null;
    const storage = (a.query as Record<string, Record<string, unknown>>)[
      p
    ] as Record<string, unknown>;
    const attestations = storage?.attestations ?? storage?.Attestations;
    if (!attestations || typeof attestations !== "function") return null;

    const digestBytes = digestHex.startsWith("0x")
      ? digestHex.slice(2)
      : digestHex;
    const digest = "0x" + digestBytes.padStart(64, "0").slice(-64);

    const result =
      await (attestations as (key1: number, key2: string) => Promise<unknown>)(
        chainKey,
        digest,
      );
    if (!result) return null;

    const human = (result as { toHuman?: () => unknown }).toHuman?.() as
      | Record<string, unknown>
      | undefined;
    if (!human) return null;

    const attestation = human.attestation as
      | Record<string, unknown>
      | undefined;
    if (!attestation) return null;

    const headerNumber = parseBlockNumber(
      attestation.headerNumber ?? attestation.header_number ?? 0,
    );
    const headerHash = String(
      attestation.headerHash ?? attestation.header_hash ?? "",
    ).replace(/^0x/, "");
    const root = String(attestation.root ?? "").replace(/^0x/, "");
    const prevDigest = attestation.prevDigest ?? attestation.prev_digest;
    const digestVal = attestation.digest ?? human.digest;

    return {
      headerNumber,
      headerHash,
      root,
      prevDigest: prevDigest != null
        ? String(prevDigest).replace(/^0x/, "")
        : undefined,
      digest: digestVal != null ? String(digestVal).replace(/^0x/, "") : "",
    };
  } catch (e) {
    if (verboseLogging) console.warn("[usc] error:", e);
    return null;
  }
}
