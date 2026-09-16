// History screen data source: client-side getLogs against every configured spoke's BridgeVault —
// no indexer, since this is a handful of rows for one connected wallet, not org-wide analytics.
// Deposited is matched by depositor==address OR recipient==address (two separate indexed topics,
// so two getLogs calls per chain); status is "released" once a matching Released log is found on
// the deposit's destination vault (recipient+token+amount — see use-transfer-status.ts for why this
// heuristic, rather than an exact messageId join, is the deliberate choice here).
//
// Scan cursors (last block fully scanned per chain+address) are cached in localStorage purely to
// avoid re-scanning from genesis on every visit; chain state is still the source of truth; a wiped
// cache just costs one slower re-scan, never a wrong answer.
//
// The cursor alone isn't enough, though: once it's advanced past a deposit's block, re-scanning
// only the *new* range after it would never see that deposit again. So the deposits found in past
// scans are cached alongside the cursor (same "best-effort, chain is still the source of truth"
// rule — a wiped cache just re-discovers everything from genesisBlock) and merged with each new
// incremental scan, rather than each scan being treated as the full answer on its own.
import { useConfig } from "wagmi";
import { getPublicClient } from "wagmi/actions";
import { useQuery } from "@tanstack/react-query";
import type { PublicClient } from "viem";

import { BridgeVaultAbi } from "@/lib/contracts/generated/bridge-vault.abi";
import { SPOKE_CHAINS, spokeByChainKey, type SpokeChain } from "@/lib/chains";

const CHUNK_BLOCKS = 2_000n;
const RELEASE_LOOKBACK_BLOCKS = 5_000n;

export interface TransferRow {
  id: string; // depositTxHash-logIndex, stable row key
  sourceChainKey: number;
  destChainKey: number;
  depositTxHash: `0x${string}`;
  depositor: `0x${string}`;
  recipient: `0x${string}`;
  token: `0x${string}`;
  amount: bigint;
  nonce: bigint;
  status: "pending" | "released";
  releasedTxHash?: `0x${string}`;
}

const DEPOSITED_EVENT = {
  type: "event",
  name: "Deposited",
  inputs: [
    { name: "nonce", type: "uint256", indexed: true },
    { name: "depositor", type: "address", indexed: true },
    { name: "recipient", type: "address", indexed: true },
    { name: "destChainKey", type: "uint32", indexed: false },
    { name: "token", type: "address", indexed: false },
    { name: "amount", type: "uint256", indexed: false },
  ],
} as const;

const RELEASED_EVENT = BridgeVaultAbi.find(
  (item) => item.type === "event" && item.name === "Released",
) as Extract<(typeof BridgeVaultAbi)[number], { type: "event" }>;

function cursorKey(chainKey: number, address: string): string {
  // v2: versioned so a cursor written by the pre-known-rows-cache code (which could advance past a
  // deposit's block with nothing else remembering it, silently dropping it from History on the next
  // visit) is ignored rather than trusted — see the header comment above.
  return `usc-bridge-dapp:scan-cursor:v2:${chainKey}:${address.toLowerCase()}`;
}

function readCursor(chainKey: number, address: string): bigint | undefined {
  try {
    const raw = localStorage.getItem(cursorKey(chainKey, address));
    return raw ? BigInt(raw) : undefined;
  } catch {
    return undefined;
  }
}

function writeCursor(chainKey: number, address: string, block: bigint): void {
  try {
    localStorage.setItem(cursorKey(chainKey, address), block.toString());
  } catch {
    // best-effort; a private window or full storage just costs a re-scan next time
  }
}

// Deposits found in past scans, persisted alongside the cursor — see the header comment above for
// why the cursor alone isn't a sufficient cache. `status`/`releasedTxHash` are deliberately not
// persisted: they're recomputed fresh on every load (see useBridgeHistory's withStatus step) since
// a deposit can transition from "pending" to "released" between visits.
type StoredRow = Omit<
  TransferRow,
  "amount" | "nonce" | "status" | "releasedTxHash"
> & {
  amount: string;
  nonce: string;
};

function knownRowsKey(chainKey: number, address: string): string {
  return `usc-bridge-dapp:known-rows:${chainKey}:${address.toLowerCase()}`;
}

function readKnownRows(chainKey: number, address: string): TransferRow[] {
  try {
    const raw = localStorage.getItem(knownRowsKey(chainKey, address));
    if (!raw) return [];
    const stored = JSON.parse(raw) as StoredRow[];
    return stored.map((row) => ({
      ...row,
      amount: BigInt(row.amount),
      nonce: BigInt(row.nonce),
      status: "pending",
    }));
  } catch {
    return [];
  }
}

function writeKnownRows(
  chainKey: number,
  address: string,
  rows: TransferRow[],
): void {
  try {
    const stored: StoredRow[] = rows.map((row) => ({
      id: row.id,
      sourceChainKey: row.sourceChainKey,
      destChainKey: row.destChainKey,
      depositTxHash: row.depositTxHash,
      depositor: row.depositor,
      recipient: row.recipient,
      token: row.token,
      amount: row.amount.toString(),
      nonce: row.nonce.toString(),
    }));
    localStorage.setItem(
      knownRowsKey(chainKey, address),
      JSON.stringify(stored),
    );
  } catch {
    // best-effort; a private window or full storage just costs re-discovering everything next time
  }
}

async function scanChunked(
  client: PublicClient,
  spoke: SpokeChain,
  address: `0x${string}`,
): Promise<TransferRow[]> {
  const tip = await client.getBlockNumber();
  const cached = readCursor(spoke.chainKey, address);
  let fromBlock = cached !== undefined ? cached + 1n : spoke.genesisBlock;
  const newRows: TransferRow[] = [];

  const seenDepositTx = new Set<string>();

  while (fromBlock <= tip) {
    const toBlock =
      fromBlock + CHUNK_BLOCKS > tip ? tip : fromBlock + CHUNK_BLOCKS;

    const [byDepositor, byRecipient] = await Promise.all([
      client.getLogs({
        address: spoke.bridgeVaultAddress,
        event: DEPOSITED_EVENT,
        args: { depositor: address },
        fromBlock,
        toBlock,
      }),
      client.getLogs({
        address: spoke.bridgeVaultAddress,
        event: DEPOSITED_EVENT,
        args: { recipient: address },
        fromBlock,
        toBlock,
      }),
    ]);

    for (const log of [...byDepositor, ...byRecipient]) {
      const rowId = `${log.transactionHash}-${log.logIndex}`;
      if (seenDepositTx.has(rowId)) continue;
      seenDepositTx.add(rowId);
      newRows.push({
        id: rowId,
        sourceChainKey: spoke.chainKey,
        destChainKey: log.args.destChainKey as number,
        depositTxHash: log.transactionHash,
        depositor: log.args.depositor as `0x${string}`,
        recipient: log.args.recipient as `0x${string}`,
        token: log.args.token as `0x${string}`,
        amount: log.args.amount as bigint,
        nonce: log.args.nonce as bigint,
        status: "pending",
      });
    }

    fromBlock = toBlock + 1n;
  }

  writeCursor(spoke.chainKey, address, tip);

  const known = readKnownRows(spoke.chainKey, address);
  const merged = [...known];
  const knownIds = new Set(known.map((row) => row.id));
  for (const row of newRows) {
    if (!knownIds.has(row.id)) merged.push(row);
  }
  writeKnownRows(spoke.chainKey, address, merged);
  return merged;
}

async function findRelease(
  client: PublicClient,
  destSpoke: SpokeChain,
  recipient: `0x${string}`,
  token: `0x${string}`,
  amount: bigint,
): Promise<`0x${string}` | undefined> {
  const tip = await client.getBlockNumber();
  const fromBlock =
    tip > RELEASE_LOOKBACK_BLOCKS ? tip - RELEASE_LOOKBACK_BLOCKS : 0n;
  const logs = await client.getLogs({
    address: destSpoke.bridgeVaultAddress,
    event: RELEASED_EVENT,
    args: { recipient },
    fromBlock,
    toBlock: "latest",
  });
  const match = logs.find(
    (log) =>
      (
        log.args as { token?: `0x${string}`; amount?: bigint }
      ).token?.toLowerCase() === token.toLowerCase() &&
      (log.args as { token?: `0x${string}`; amount?: bigint }).amount ===
        amount,
  );
  return match?.transactionHash;
}

export function useBridgeHistory(address: `0x${string}` | undefined) {
  const config = useConfig();

  const query = useQuery({
    queryKey: ["bridge-history", address],
    queryFn: async (): Promise<TransferRow[]> => {
      const clientFor = (chainId: number): PublicClient | undefined =>
        getPublicClient(config, { chainId }) as PublicClient | undefined;

      const perChain = await Promise.all(
        SPOKE_CHAINS.map((spoke) => {
          const client = clientFor(spoke.chain.id);
          return client
            ? scanChunked(client, spoke, address as `0x${string}`)
            : Promise.resolve([]);
        }),
      );
      const deposits = perChain.flat();

      const withStatus = await Promise.all(
        deposits.map(async (row) => {
          const destSpoke = spokeByChainKey(row.destChainKey);
          const destClient = destSpoke
            ? clientFor(destSpoke.chain.id)
            : undefined;
          if (!destSpoke || !destClient) return row;
          const releasedTxHash = await findRelease(
            destClient,
            destSpoke,
            row.recipient,
            row.token,
            row.amount,
          );
          return releasedTxHash
            ? { ...row, status: "released" as const, releasedTxHash }
            : row;
        }),
      );

      return withStatus.sort((a, b) =>
        a.depositTxHash < b.depositTxHash ? 1 : -1,
      );
    },
    enabled: Boolean(address),
  });

  return { rows: query.data ?? [], isLoading: query.isLoading };
}
