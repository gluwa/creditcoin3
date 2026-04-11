/**
 * Event listeners for Outbox (source chain) and Inbox (destination chain) contracts.
 * Uses polling via queryFilter to avoid WebSocket requirements.
 */

import { ethers } from "ethers";
import type { PublishedMessage, DeliveredMessage } from "./types.js";

const LOG_OUTBOX = "[Outbox listener]";
const LOG_INBOX = "[Inbox listener]";
const EVENT_MESSAGE_PUBLISHED = "MessagePublished";
const EVENT_MESSAGE_DELIVERED = "MessageDelivered";

const OUTBOX_ABI = [
  "event MessagePublished(bytes32 indexed messageId, bytes32 indexed emitterAddress, bool requiresAck, bytes payload)",
];

const INBOX_ABI = [
  "event MessageDelivered(bytes32 indexed messageId, address indexed processor)",
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

/**
 * Polls the Inbox contract for MessageDelivered events.
 * Calls `onDelivered` for each new event found.
 * Returns a function to stop the polling loop.
 */
export function listenInbox(
  provider: ethers.JsonRpcProvider,
  inboxAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  onDelivered: (msg: DeliveredMessage) => void | Promise<void>,
): StopFn {
  const contract = new ethers.Contract(inboxAddress, INBOX_ABI, provider);
  let lastBlock = fromBlock;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout>;

  const poll = async () => {
    if (stopped) return;
    try {
      const latest = await provider.getBlockNumber();
      if (latest > lastBlock) {
        const events = await contract.queryFilter(
          EVENT_MESSAGE_DELIVERED,
          lastBlock + 1,
          latest,
        );
        for (const event of events) {
          const log = event as ethers.EventLog;
          const delivered: DeliveredMessage = {
            messageId: log.args[0] as string,
            processor: log.args[1] as string,
          };
          await onDelivered(delivered);
        }
        lastBlock = latest;
      }
    } catch (err) {
      console.error(`${LOG_INBOX} poll error:`, err);
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
