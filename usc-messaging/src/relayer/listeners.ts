/**
 * Event listener for the SimpleInbox (destination chain) contract.
 * Uses polling via queryFilter to avoid WebSocket requirements.
 */

import { ethers } from "ethers";

const LOG_INBOX = "[Inbox listener]";
const LOG_RELAYER = "[Relayer listener]";
const EVENT_MESSAGE_DELIVERED = "MessageDelivered";
const EVENT_MESSAGE_PAID = "MessagePaid";

const INBOX_ABI = ["event MessageDelivered(bytes32 indexed messageId)"];
const RELAYER_ABI = ["event MessagePaid(bytes32 indexed messageId)"];

export type StopFn = () => void;

/**
 * Polls the SimpleInbox contract for MessageDelivered events.
 * Calls `onDelivered` for each new event found.
 * Returns a function to stop the polling loop.
 */
export function listenInbox(
  provider: ethers.JsonRpcProvider,
  inboxAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  onDelivered: (messageId: string) => void | Promise<void>,
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
          const messageId = log.args[0] as string;
          await onDelivered(messageId);
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

/**
 * Polls the SimpleRelayer contract for MessagePaid events.
 * Calls `onPaid` for each new event found.
 * Returns a function to stop the polling loop.
 */
export function listenRelayer(
  provider: ethers.JsonRpcProvider,
  relayerAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  onPaid: (messageId: string) => void | Promise<void>,
): StopFn {
  const contract = new ethers.Contract(relayerAddress, RELAYER_ABI, provider);
  let lastBlock = fromBlock;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout>;

  const poll = async () => {
    if (stopped) return;
    try {
      const latest = await provider.getBlockNumber();
      if (latest > lastBlock) {
        const events = await contract.queryFilter(
          EVENT_MESSAGE_PAID,
          lastBlock + 1,
          latest,
        );
        for (const event of events) {
          const log = event as ethers.EventLog;
          const messageId = log.args[0] as string;
          await onPaid(messageId);
        }
        lastBlock = latest;
      }
    } catch (err) {
      console.error(`${LOG_RELAYER} poll error:`, err);
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
