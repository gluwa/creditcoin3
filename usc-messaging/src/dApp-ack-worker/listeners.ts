import { ethers } from "ethers";

export const EVENT_TOKENS_BRIDGED = "TokensBridged";
export const EVENT_TOKENS_BURNED_FOR_BRIDGING = "TokensBurnedForBridging";

const DESTINATION_CONTRACT_ABI = [
  "event TokensBridged(bytes32 indexed messageId, address indexed emitterAddress, address indexed recipient, uint256 amount)",
  "event TokensBurnedForBridging(address indexed from, uint256 amount)",
];

export interface TokensBridgedEvent {
  eventName: typeof EVENT_TOKENS_BRIDGED;
  messageId: string;
  emitterAddress: string;
  recipient: string;
  amount: string;
  txHash?: string;
  blockNumber?: number;
}

export interface TokensBurnedForBridgingEvent {
  eventName: typeof EVENT_TOKENS_BURNED_FOR_BRIDGING;
  from: string;
  amount: string;
  txHash?: string;
  blockNumber?: number;
}

export type DestinationEvent =
  | TokensBridgedEvent
  | TokensBurnedForBridgingEvent;

export type StopFn = () => void;

export function listenDestinationContract(
  provider: ethers.JsonRpcProvider,
  destinationContractAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  onEvent: (event: DestinationEvent) => void | Promise<void>,
): StopFn {
  const contract = new ethers.Contract(
    destinationContractAddress,
    DESTINATION_CONTRACT_ABI,
    provider,
  );

  let lastBlock = fromBlock;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const poll = async () => {
    if (stopped) return;

    try {
      const latest = await provider.getBlockNumber();

      if (latest > lastBlock) {
        const bridgedFilter = contract.filters.TokensBridged();
        const burnedFilter = contract.filters.TokensBurnedForBridging();

        const [bridgedEvents, burnedEvents] = await Promise.all([
          contract.queryFilter(bridgedFilter, lastBlock + 1, latest),
          contract.queryFilter(burnedFilter, lastBlock + 1, latest),
        ]);

        const allEvents = [...bridgedEvents, ...burnedEvents].sort(
          (a, b) => a.blockNumber - b.blockNumber || a.index - b.index,
        );

        for (const event of allEvents) {
          const log = event as ethers.EventLog;

          if (log.fragment.name === EVENT_TOKENS_BRIDGED) {
            const parsed: TokensBridgedEvent = {
              eventName: EVENT_TOKENS_BRIDGED,
              messageId: String(log.args[0]),
              emitterAddress: String(log.args[1]),
              recipient: String(log.args[2]),
              amount: log.args[3].toString(),
              txHash: log.transactionHash,
              blockNumber: log.blockNumber,
            };

            await onEvent(parsed);
          } else if (
            log.fragment.name === EVENT_TOKENS_BURNED_FOR_BRIDGING
          ) {
            const parsed: TokensBurnedForBridgingEvent = {
              eventName: EVENT_TOKENS_BURNED_FOR_BRIDGING,
              from: String(log.args[0]),
              amount: log.args[1].toString(),
              txHash: log.transactionHash,
              blockNumber: log.blockNumber,
            };

            await onEvent(parsed);
          }
        }

        lastBlock = latest;
      }
    } catch (err) {
      console.error("[Destination Listener] poll error:", err);
    }

    if (!stopped) {
      timer = setTimeout(poll, pollIntervalMs);
    }
  };

  void poll();

  return () => {
    stopped = true;
    if (timer) clearTimeout(timer);
  };
}
