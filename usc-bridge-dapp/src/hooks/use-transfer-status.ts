// Composed transfer state machine shared by the progress and claim screens. Reload-safe by design:
// given only the depositTxHash from the URL, it locates which configured spoke the tx belongs to
// and rebuilds the whole picture from chain state — no wallet, no query param, no localStorage
// required.
//
// Step 4 ("released") is the only authoritative "done" signal (lock-and-release means nothing after
// a successful deposit is ever silently lost, only possibly delayed) and deliberately does NOT
// depend on step 3: it matches BridgeVault.Released on the destination directly by
// recipient+token+amount (all known from the deposit itself), so it stays correct even with
// NEXT_PUBLIC_BRIDGE_HUB_ADDRESS unset. Step 3 ("claimed") is a best-effort enrichment on top —
// reads BridgeHub.claimed(key) with the exact same key the claimer bot computes — and silently
// reports `undefined` (not false) when Creditcoin access isn't configured.
import { useMemo } from "react";
import { useConfig, usePublicClient } from "wagmi";
import { getPublicClient } from "wagmi/actions";
import { useQuery } from "@tanstack/react-query";
import type { AbiEvent, Log, PublicClient } from "viem";
import {
  decodeEventLog,
  encodeAbiParameters,
  keccak256,
  parseAbiParameters,
  toHex,
} from "viem";

import { BridgeVaultAbi } from "@/lib/contracts/generated/bridge-vault.abi";
import { BridgeHubAbi } from "@/lib/contracts/generated/bridge-hub.abi";
import { SPOKE_CHAINS, spokeByChainKey } from "@/lib/chains";
import {
  BRIDGE_HUB_ADDRESS,
  creditcoinPublicClient,
} from "@/lib/contracts/hub";
import { useAttestedHeight } from "./use-attested-height";

// Selectors for BridgeVault's own custom errors — computed the same way Solidity does (first 4
// bytes of keccak256 of the error signature) so a DestinationDeliveryFailed's raw `failureSelector`
// can be shown as a readable reason instead of an opaque 4-byte hex value. Any selector not listed
// here (e.g. a revert from somewhere other than BridgeVault) just falls back to the raw hex.
const KNOWN_FAILURE_REASONS: Record<string, string> = {
  [keccak256(toHex("InsufficientLiquidity(address,uint256,uint256)")).slice(
    0,
    10,
  )]: "Insufficient liquidity in the destination vault",
  [keccak256(toHex("UntrustedEmitter(address,address)")).slice(0, 10)]:
    "Untrusted emitter — release message did not originate from the configured BridgeHub",
  [keccak256(toHex("MessageAlreadyProcessed(bytes32)")).slice(0, 10)]:
    "Message already processed (replay guard)",
  [keccak256(toHex("NativeReleaseFailed(address,uint256)")).slice(0, 10)]:
    "Native transfer to the recipient failed",
  [keccak256(toHex("UnsupportedToken(address)")).slice(0, 10)]:
    "Unsupported token",
};

const DESTINATION_DELIVERY_FAILED_EVENT = {
  type: "event",
  name: "DestinationDeliveryFailed",
  inputs: [
    { name: "messageId", type: "bytes32", indexed: true },
    { name: "destination", type: "address", indexed: true },
    { name: "failureSelector", type: "bytes4", indexed: false },
  ],
} as const;

// Both the Creditcoin RPC and the spoke chains' public RPCs cap eth_getLogs at a 2048-block range —
// chunk any lookback wider than that. Run in parallel (not the sequential walk use-bridge-history.ts
// uses for a durable cursor-based scan): this is a bounded, best-effort recent-history check, not a
// resumable one, so wall-clock time matters more than request count.
const LOG_CHUNK_BLOCKS = 2_000n;

interface LogScanParams {
  address: `0x${string}`;
  event: AbiEvent;
  args?: Record<string, unknown>;
}

/** A getLogs result decoded against a specific event: unlike the base `Log` type, this carries
 *  `args` keyed by the event's parameter names (both indexed and non-indexed). */
type DecodedLog = Log & { args: Record<string, unknown> };

async function scanRecentLogs(
  client: PublicClient,
  lookbackBlocks: bigint,
  params: LogScanParams,
): Promise<DecodedLog[]> {
  const tip = await client.getBlockNumber();
  const from = tip > lookbackBlocks ? tip - lookbackBlocks : 0n;
  const chunkPromises: Promise<DecodedLog[]>[] = [];
  for (let start = from; start <= tip; start += LOG_CHUNK_BLOCKS + 1n) {
    const end = start + LOG_CHUNK_BLOCKS > tip ? tip : start + LOG_CHUNK_BLOCKS;
    chunkPromises.push(
      client.getLogs({
        ...params,
        fromBlock: start,
        toBlock: end,
      } as Parameters<typeof client.getLogs>[0]) as Promise<DecodedLog[]>,
    );
  }
  return (await Promise.all(chunkPromises)).flat();
}

/** Tries every configured spoke's RPC for `depositTxHash` and returns the chain key of whichever
 *  one has it — the only piece of state a reload can't get from the URL alone. Uses the plain
 *  `getPublicClient` action (not the `usePublicClient` hook) since the set of spokes to check is
 *  dynamic — hooks can't be called from a loop/map. */
function useLocateSourceChainKey(depositTxHash: `0x${string}` | undefined) {
  const config = useConfig();

  return useQuery({
    queryKey: ["locate-source-chain", depositTxHash],
    queryFn: async (): Promise<number | null> => {
      if (!depositTxHash) return null;
      const results = await Promise.allSettled(
        SPOKE_CHAINS.map(async (spoke) => {
          const client = getPublicClient(config, { chainId: spoke.chain.id });
          if (!client) throw new Error("no client");
          await client.getTransactionReceipt({ hash: depositTxHash });
          return spoke.chainKey;
        }),
      );
      const found = results.find((r) => r.status === "fulfilled") as
        PromiseFulfilledResult<number> | undefined;
      return found?.value ?? null;
    },
    enabled: Boolean(depositTxHash),
    retry: 3,
  });
}

// Bounds the destination-side Released scan to a recent, real-time window — most public RPCs
// reject an unbounded fromBlock=earliest getLogs on a chain with millions of blocks. Good enough
// for "watch this happen live"; a stale reload well outside this window should use the History
// screen's durable chunked scan instead.
const RELEASE_SCAN_LOOKBACK_BLOCKS = 5_000n;

// Bounds the Claimed-event lookup (Creditcoin) and the DestinationDeliveryFailed lookup (dest
// spoke) to a recent window, same rationale as RELEASE_SCAN_LOOKBACK_BLOCKS below — a claim that's
// older than this just won't show enriched "claimed"/"failed" detail, it doesn't affect
// correctness of the authoritative deposited/released steps.
const CLAIM_EVENT_LOOKBACK_BLOCKS = 50_000n;

export type TransferStep =
  "loading" | "not-found" | "deposited" | "attested" | "released" | "failed";

export interface DepositedInfo {
  nonce: bigint;
  depositor: `0x${string}`;
  recipient: `0x${string}`;
  destChainKey: number;
  token: `0x${string}`;
  amount: bigint;
  blockNumber: bigint;
}

export interface DeliveryFailure {
  /** The 4-byte selector BridgeVault's (or another destination contract's) revert carried. */
  selector: `0x${string}`;
  /** Decoded from KNOWN_FAILURE_REASONS when recognized, else the raw selector. */
  reason: string;
  /** The relayer's deliverMessage tx on the destination chain that recorded the failure. */
  txHash: `0x${string}`;
}

export interface TransferStatus {
  step: TransferStep;
  sourceChainKey?: number;
  deposit?: DepositedInfo;
  attestedHeight?: number | null;
  /** undefined = unknown (BridgeHub read not configured), not "not yet claimed". */
  claimed?: boolean;
  /** The BridgeHub.claim tx that published the release message, once found. */
  claimTxHash?: `0x${string}`;
  releasedTxHash?: `0x${string}`;
  /** Set once a terminal DestinationDeliveryFailed is found for this deposit's release message —
   *  see BridgeVault.sol's NatSpec: there is no on-chain retry path once this happens. */
  failure?: DeliveryFailure;
}

export function useTransferStatus(
  depositTxHash: `0x${string}` | undefined,
): TransferStatus {
  const locateQuery = useLocateSourceChainKey(depositTxHash);
  const sourceChainKey = locateQuery.data ?? undefined;
  const source =
    sourceChainKey !== undefined ? spokeByChainKey(sourceChainKey) : undefined;
  const sourceClient = usePublicClient({ chainId: source?.chain.id });

  const depositQuery = useQuery({
    queryKey: ["deposit-receipt", sourceChainKey, depositTxHash],
    queryFn: async (): Promise<DepositedInfo | null> => {
      if (!sourceClient || !depositTxHash || !source) return null;
      const receipt = await sourceClient.getTransactionReceipt({
        hash: depositTxHash,
      });
      for (const log of receipt.logs) {
        if (
          log.address.toLowerCase() !== source.bridgeVaultAddress.toLowerCase()
        )
          continue;
        try {
          const decoded = decodeEventLog({
            abi: BridgeVaultAbi,
            data: log.data,
            topics: log.topics,
            eventName: "Deposited",
          });
          return {
            nonce: decoded.args.nonce,
            depositor: decoded.args.depositor,
            recipient: decoded.args.recipient,
            destChainKey: decoded.args.destChainKey,
            token: decoded.args.token,
            amount: decoded.args.amount,
            blockNumber: receipt.blockNumber,
          };
        } catch {
          continue;
        }
      }
      return null;
    },
    enabled: Boolean(sourceClient && depositTxHash && source),
    retry: 3,
  });

  const deposit = depositQuery.data ?? undefined;
  const { attestedHeight, isAttested } = useAttestedHeight(
    sourceChainKey,
    deposit?.blockNumber,
  );

  const claimKey = useMemo(() => {
    if (!source || deposit === undefined || sourceChainKey === undefined)
      return undefined;
    return keccak256(
      encodeAbiParameters(parseAbiParameters("uint32, address, uint256"), [
        sourceChainKey,
        source.bridgeVaultAddress,
        deposit.nonce,
      ]),
    );
  }, [source, deposit, sourceChainKey]);

  const claimedQuery = useQuery({
    queryKey: ["hub-claimed", claimKey],
    queryFn: async (): Promise<boolean> => {
      if (!creditcoinPublicClient || !BRIDGE_HUB_ADDRESS || !claimKey)
        return false;
      return creditcoinPublicClient.readContract({
        address: BRIDGE_HUB_ADDRESS,
        abi: BridgeHubAbi,
        functionName: "claimed",
        args: [claimKey],
      });
    },
    enabled: Boolean(
      creditcoinPublicClient && BRIDGE_HUB_ADDRESS && isAttested && claimKey,
    ),
    refetchInterval: (q) => (q.state.data ? false : 15_000),
  });

  // Enrichment on top of claimedQuery's plain boolean: fetches the actual Claimed log to recover
  // its tx hash (for step detail) and the real dispatcher messageId (Outbox.publishMessage's
  // return value — distinct from claimKey, which is what BridgeHub itself uses as a claimed-lookup
  // key) needed to look up a terminal delivery failure below.
  const claimedEventQuery = useQuery({
    queryKey: ["hub-claimed-event", claimKey],
    queryFn: async (): Promise<{
      txHash: `0x${string}`;
      messageId: `0x${string}`;
    } | null> => {
      if (!creditcoinPublicClient || !BRIDGE_HUB_ADDRESS || !claimKey)
        return null;
      const claimedAbiItem = BridgeHubAbi.find(
        (item) => item.type === "event" && item.name === "Claimed",
      );
      const logs = await scanRecentLogs(
        creditcoinPublicClient,
        CLAIM_EVENT_LOOKBACK_BLOCKS,
        {
          address: BRIDGE_HUB_ADDRESS,
          event: claimedAbiItem as Extract<
            (typeof BridgeHubAbi)[number],
            { type: "event" }
          >,
          args: { key: claimKey },
        },
      );
      const log = logs[0];
      if (!log) return null;
      return {
        txHash: log.transactionHash as `0x${string}`,
        messageId: (log.args as { messageId?: `0x${string}` }).messageId!,
      };
    },
    enabled: Boolean(
      creditcoinPublicClient &&
      BRIDGE_HUB_ADDRESS &&
      claimKey &&
      claimedQuery.data === true,
    ),
  });

  const destSpoke = deposit ? spokeByChainKey(deposit.destChainKey) : undefined;
  const destClient = usePublicClient({ chainId: destSpoke?.chain.id });

  const releasedQuery = useQuery({
    queryKey: [
      "released",
      deposit?.destChainKey,
      destSpoke?.bridgeVaultAddress,
      deposit?.recipient,
      deposit?.token,
      deposit?.amount?.toString(),
    ],
    queryFn: async (): Promise<`0x${string}` | null> => {
      if (!destClient || !destSpoke || !deposit) return null;
      const tip = await destClient.getBlockNumber();
      const fromBlock =
        tip > RELEASE_SCAN_LOOKBACK_BLOCKS
          ? tip - RELEASE_SCAN_LOOKBACK_BLOCKS
          : 0n;
      const releasedAbiItem = BridgeVaultAbi.find(
        (item) => item.type === "event" && item.name === "Released",
      );
      const logs = await destClient.getLogs({
        address: destSpoke.bridgeVaultAddress,
        event: releasedAbiItem as Extract<
          (typeof BridgeVaultAbi)[number],
          { type: "event" }
        >,
        args: { recipient: deposit.recipient },
        fromBlock,
        toBlock: "latest",
      });
      const match = logs.find(
        (log) =>
          (
            log.args as { token?: `0x${string}`; amount?: bigint }
          ).token?.toLowerCase() === deposit.token.toLowerCase() &&
          (log.args as { token?: `0x${string}`; amount?: bigint }).amount ===
            deposit.amount,
      );
      return match?.transactionHash ?? null;
    },
    enabled: Boolean(destClient && destSpoke && deposit && isAttested),
    refetchInterval: (q) => (q.state.data ? false : 10_000),
  });

  const releasedTxHash = releasedQuery.data ?? undefined;
  const messageId = claimedEventQuery.data?.messageId;

  // Checked only once claimed, not yet released, and a router address is configured for the
  // destination spoke — a delivery can't have failed before it was claimed, and if it already
  // released there's nothing to explain.
  const failureQuery = useQuery({
    queryKey: ["delivery-failed", destSpoke?.chainKey, messageId],
    queryFn: async (): Promise<DeliveryFailure | null> => {
      if (!destClient || !destSpoke?.dispatcherRouterAddress || !messageId)
        return null;
      const logs = await scanRecentLogs(
        destClient,
        CLAIM_EVENT_LOOKBACK_BLOCKS,
        {
          address: destSpoke.dispatcherRouterAddress,
          event: DESTINATION_DELIVERY_FAILED_EVENT,
          args: { messageId },
        },
      );
      const log = logs[0];
      if (!log) return null;
      const selector = (log.args as { failureSelector?: `0x${string}` })
        .failureSelector!;
      return {
        selector,
        reason:
          KNOWN_FAILURE_REASONS[selector] ?? `Unknown error (${selector})`,
        txHash: log.transactionHash as `0x${string}`,
      };
    },
    enabled: Boolean(
      destClient &&
      destSpoke?.dispatcherRouterAddress &&
      messageId &&
      !releasedTxHash,
    ),
    refetchInterval: (q) => (q.state.data ? false : 15_000),
  });

  if (locateQuery.isLoading || depositQuery.isLoading)
    return { step: "loading" };
  if (locateQuery.data === null || !deposit) return { step: "not-found" };

  const claimed = claimedQuery.data;
  const failure = failureQuery.data ?? undefined;

  let step: TransferStep = "deposited";
  if (isAttested) step = "attested";
  if (failure) step = "failed";
  if (releasedTxHash) step = "released";

  return {
    step,
    sourceChainKey,
    deposit,
    attestedHeight,
    claimed,
    claimTxHash: claimedEventQuery.data?.txHash,
    releasedTxHash,
    failure,
  };
}
