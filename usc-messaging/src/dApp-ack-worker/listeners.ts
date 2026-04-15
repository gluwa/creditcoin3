import { ethers } from "ethers";

export const EVENT_MESSAGE_RECEIVED = "MessageReceived";

const DESTINATION_CONTRACT_ABI = [
  "event MessageReceived(bytes32 indexed messageId, address indexed emitter, bytes payload)",
];

export interface ReceivedMessage {
  messageId: string;
  emitter: string;
  payload: string;
  txHash?: string;
}

export type StopFn = () => void;

export function listenDestinationContract(
  provider: ethers.JsonRpcProvider,
  destinationContractAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  onMessage: (msg: ReceivedMessage) => void | Promise<void>,
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
        const filter = contract.filters.MessageReceived();
        const events = await contract.queryFilter(filter, lastBlock + 1, latest);

        for (const event of events) {
          const log = event as ethers.EventLog;

          const msg: ReceivedMessage = {
            messageId: String(log.args[0]),
            emitter: String(log.args[1]),
            payload: String(log.args[2]),
            txHash: log.transactionHash,
          };

          await onMessage(msg);
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