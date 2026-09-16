// Shared by use-transfer-status.ts (progress page) and use-bridge-history.ts (History table) —
// both need the same answer to "has this deposit's BridgeVault.Released fired on the destination
// chain yet," matched by recipient+token+amount (a deliberate heuristic, not an exact messageId
// join — see use-transfer-status.ts's header comment for why).
//
// Anchored at the destination vault's genesis block, not a fixed recent lookback: a release can
// land arbitrarily long after its deposit (this is the authoritative "done" signal, so missing an
// old one is a correctness bug, not just a UX rough edge — this is exactly the bug that made both
// the progress page and History show "pending"/stuck for already-released transfers). The scan
// cursor below, keyed by depositTxHash (already the stable per-transfer identity both callers
// have), keeps repeat lookups cheap without needing to remember the actual result: self-healing
// like every other localStorage cache in this app — a wiped cache just costs one wider re-scan.
import type { Log, PublicClient } from "viem";

import { BridgeVaultAbi } from "@/lib/contracts/generated/bridge-vault.abi";
import type { SpokeChain } from "@/lib/chains";

const LOG_CHUNK_BLOCKS = 2_000n;

const RELEASED_EVENT = BridgeVaultAbi.find(
  (item) => item.type === "event" && item.name === "Released",
) as Extract<(typeof BridgeVaultAbi)[number], { type: "event" }>;

type DecodedLog = Log & {
  args: { token?: `0x${string}`; amount?: bigint };
};

function cursorKey(depositTxHash: string): string {
  return `usc-bridge-dapp:release-scan-cursor:v1:${depositTxHash.toLowerCase()}`;
}

function readCursor(depositTxHash: string): bigint | undefined {
  try {
    const raw = localStorage.getItem(cursorKey(depositTxHash));
    return raw ? BigInt(raw) : undefined;
  } catch {
    return undefined;
  }
}

function writeCursor(depositTxHash: string, block: bigint): void {
  try {
    localStorage.setItem(cursorKey(depositTxHash), block.toString());
  } catch {
    // best-effort; a wiped/unavailable cache just costs a wider re-scan next time
  }
}

/** Looks for a BridgeVault.Released log on `destSpoke` matching `recipient`/`token`/`amount`.
 *  Returns the release's tx hash once found, else undefined (not yet released, as far as this
 *  scan has checked). `depositTxHash` keys the persisted scan cursor — pass the depositing tx's
 *  own hash, which is unique per transfer regardless of how many share the same recipient. */
export async function findRelease(
  client: PublicClient,
  destSpoke: SpokeChain,
  depositTxHash: string,
  recipient: `0x${string}`,
  token: `0x${string}`,
  amount: bigint,
): Promise<`0x${string}` | undefined> {
  const tip = await client.getBlockNumber();
  const cached = readCursor(depositTxHash);
  const fromBlock = cached !== undefined ? cached + 1n : destSpoke.genesisBlock;
  if (fromBlock > tip) return undefined;

  const chunkPromises: Promise<DecodedLog[]>[] = [];
  for (let start = fromBlock; start <= tip; start += LOG_CHUNK_BLOCKS + 1n) {
    const end = start + LOG_CHUNK_BLOCKS > tip ? tip : start + LOG_CHUNK_BLOCKS;
    chunkPromises.push(
      client.getLogs({
        address: destSpoke.bridgeVaultAddress,
        event: RELEASED_EVENT,
        args: { recipient },
        fromBlock: start,
        toBlock: end,
      }) as Promise<DecodedLog[]>,
    );
  }
  const logs = (await Promise.all(chunkPromises)).flat();

  const match = logs.find(
    (log) =>
      log.args.token?.toLowerCase() === token.toLowerCase() &&
      log.args.amount === amount,
  );
  if (match) return match.transactionHash as `0x${string}`;

  writeCursor(depositTxHash, tip);
  return undefined;
}
