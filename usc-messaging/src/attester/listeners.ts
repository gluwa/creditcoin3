/**
 * Event listener for the Outbox (source chain) contract.
 * Uses polling via queryFilter to avoid WebSocket requirements.
 */

import { ethers } from "ethers";
import type { PublishedMessage } from "./types.js";

const LOG_OUTBOX = "[Outbox listener]";
const EVENT_MESSAGE_PUBLISHED = "MessagePublished";

const OUTBOX_ABI = [
  "event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool requiresAck, bytes payload)",
];

export type StopFn = () => void;

/**
 * Polls the Outbox contract for MessagePublished events.
 * Calls `onMessage` for each new event found.
 * Returns a function to stop the polling loop.
 */
export function listenOutbox(
  provider: ethers.JsonRpcProvider,
  outboxAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  onMessage: (msg: PublishedMessage) => void | Promise<void>,
): StopFn {
  const contract = new ethers.Contract(outboxAddress, OUTBOX_ABI, provider);
  let lastBlock = fromBlock;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout>;

  const poll = async () => {
    if (stopped) return;
    try {
      const latest = await provider.getBlockNumber();
      if (latest > lastBlock) {
        const events = await contract.queryFilter(
          EVENT_MESSAGE_PUBLISHED,
          lastBlock + 1,
          latest,
        );
        for (const event of events) {
          const log = event as ethers.EventLog;
          const msg: PublishedMessage = {
            messageId: log.args[0] as string,
            emitterAddress: log.args[1] as string,
            requiresAck: log.args[2] as boolean,
            payload: log.args[3] as string,
          };
          await onMessage(msg);
        }
        lastBlock = latest;
      }
    } catch (err) {
      console.error(`${LOG_OUTBOX} poll error:`, err);
    }
    if (!stopped) {
      timer = setTimeout(poll, pollIntervalMs);
    }
  };

  poll();

  return () => {
    stopped = true;
    clearTimeout(timer);
  };
}
