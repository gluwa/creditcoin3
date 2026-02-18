/**
 * USC (Creditcoin3) chain client
 *
 * Queries storage for supported chains, attestations, checkpoints.
 */

import { ApiPromise, WsProvider } from "@polkadot/api";

export interface SupportedChain {
  chainId: number;
  chainName: string;
  chainKey: number;
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

export async function connect(wsUrl: string): Promise<void> {
  const provider = new WsProvider(wsUrl);
  api = await ApiPromise.create({ provider, noInitWarn: true });
  await api.isReady;
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

/**
 * Get supported chains from storage.
 * Tries common pallet names (metadata may vary).
 */
export async function getSupportedChains(): Promise<SupportedChain[]> {
  const a = getApi();
  const chains: SupportedChain[] = [];

  try {
    // Try attestation pallet - some chains store supported chains there
    const pallets = Object.keys(a.query);
    const attestationPallet = pallets.find((p) =>
      p.toLowerCase().includes("attestation")
    );
    const supportedPallet = pallets.find((p) =>
      p.toLowerCase().includes("supported")
    );

    if (supportedPallet) {
      const storage = (a.query as Record<string, Record<string, unknown>>)[
        supportedPallet
      ];
      const storageName = Object.keys(storage || {}).find((s) =>
        s.toLowerCase().includes("chain")
      );
      if (storageName && storage) {
        const entries = await (storage[storageName] as {
          entries: () => Promise<[unknown, unknown][]>;
        }).entries();
        for (const [, value] of entries) {
          try {
            const decoded = value as { toHuman?: () => unknown };
            const human = decoded?.toHuman?.() as
              | Record<string, unknown>
              | undefined;
            if (human) {
              const chainId = Number(human.chainId ?? human.chain_id ?? 0);
              const nameBytes = human.chainName ?? human.chain_name;
              const chainName = Array.isArray(nameBytes)
                ? String.fromCharCode(...(nameBytes as number[]))
                : String(nameBytes ?? "");
              if (chainId && chainName) {
                const chainKey = await getChainKey(chainId, chainName);
                if (chainKey != null) {
                  chains.push({ chainId, chainName, chainKey });
                }
              }
            }
          } catch {
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

async function getChainKey(
  chainId: number,
  chainName: string,
): Promise<number | null> {
  const a = getApi();
  try {
    const pallets = Object.keys(a.query);
    const attestationPallet = pallets.find((p) =>
      p.toLowerCase().includes("attestation")
    );
    if (!attestationPallet) return null;

    const storage =
      (a.query as Record<string, Record<string, unknown>>)[attestationPallet];
    const mapName = Object.keys(storage || {}).find((s) =>
      s.toLowerCase().includes("chain") && s.toLowerCase().includes("key")
    );
    if (!mapName || !storage) return null;

    const nameBytes = Array.from(chainName).map((c) => c.charCodeAt(0));
    const result =
      await (storage[mapName] as (a: number, b: number[]) => Promise<unknown>)(
        chainId,
        nameBytes,
      );
    const val = result as {
      toNumber?: () => number;
      unwrap?: () => { toNumber?: () => number };
    };
    const n = val?.toNumber?.() ?? val?.unwrap?.()?.toNumber?.() ?? Number(val);
    return typeof n === "number" && !isNaN(n) ? n : null;
  } catch {
    return null;
  }
}

export async function getLastDigest(chainKey: number): Promise<string | null> {
  const a = getApi();
  try {
    const pallets = Object.keys(a.query);
    const p = pallets.find((x) => x.toLowerCase().includes("attestation"));
    if (!p) return null;
    const storage = (a.query as Record<string, Record<string, unknown>>)[p];
    const lastDigest = storage?.lastDigest ?? storage?.LastDigest;
    if (!lastDigest || typeof lastDigest !== "function") return null;
    const result = await (lastDigest as (key: number) => Promise<unknown>)(
      chainKey,
    );
    if (!result) return null;
    const hex = (result as { toHex?: () => string }).toHex?.() ??
      (result as { toString: () => string }).toString?.();
    return hex ?? null;
  } catch {
    return null;
  }
}

export async function getLastCheckpoint(
  chainKey: number,
): Promise<LastCheckpoint | null> {
  const a = getApi();
  try {
    const pallets = Object.keys(a.query);
    const p = pallets.find((x) => x.toLowerCase().includes("attestation"));
    if (!p) return null;
    const storage = (a.query as Record<string, Record<string, unknown>>)[p];
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
  } catch {
    return null;
  }
}

export async function getCheckpointInterval(chainKey: number): Promise<number> {
  const a = getApi();
  try {
    const pallets = Object.keys(a.query);
    const p = pallets.find((x) => x.toLowerCase().includes("attestation"));
    if (!p) return 180;
    const storage = (a.query as Record<string, Record<string, unknown>>)[p];
    const interval = storage?.attestationCheckpointInterval ??
      storage?.AttestationCheckpointInterval;
    if (!interval || typeof interval !== "function") return 180;
    const result = await (interval as (key: number) => Promise<unknown>)(
      chainKey,
    );
    return parseBlockNumber(result ?? 180) || 180;
  } catch {
    return 180;
  }
}

export async function getAttestationInterval(
  chainKey: number,
): Promise<number> {
  const a = getApi();
  try {
    const pallets = Object.keys(a.query);
    const p = pallets.find((x) => x.toLowerCase().includes("attestation"));
    if (!p) return 10;
    const storage = (a.query as Record<string, Record<string, unknown>>)[p];
    const interval = storage?.chainAttestationInterval ??
      storage?.ChainAttestationInterval;
    if (!interval || typeof interval !== "function") return 10;
    const result = await (interval as (key: number) => Promise<unknown>)(
      chainKey,
    );
    return parseBlockNumber(result ?? 10) || 10;
  } catch {
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
  const a = getApi();
  try {
    const pallets = Object.keys(a.query);
    const p = pallets.find((x) => x.toLowerCase().includes("attestation"));
    if (!p) return null;
    const storage = (a.query as Record<string, Record<string, unknown>>)[p];
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
  } catch {
    return null;
  }
}
