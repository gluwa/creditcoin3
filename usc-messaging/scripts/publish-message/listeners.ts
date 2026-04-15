/**
 * Event listener for the simpleDApp (source chain) contract.
 * Uses polling via queryFilter to avoid WebSocket requirements.
 */

import { ethers } from "ethers";
import type { DispatchedMessage } from "./types.ts";

const LOG_DAPP = "[dApp Contract Listener]";
export const EVENT_MESSAGE_DISPATCHED = "MessageDispatched";
export const EVENT_MESSAGE_DELIVERED = "MessageDelivered";

const DAPP_CONTRACT_ABI = [
  "event MessageDelivered(bytes32 indexed messageId)",
  "event MessageDispatched(bytes32 indexed messageId)",
];

export type StopFn = () => void;

export function listenDAppContract(
  provider: ethers.JsonRpcProvider,
  dappAddress: string,
  fromBlock: number,
  pollIntervalMs: number,
  eventName: string,
  onMessage: (msg: DispatchedMessage) => void | Promise<void>,
): StopFn {
  const contract = new ethers.Contract(dappAddress, DAPP_CONTRACT_ABI, provider);
  let lastBlock = fromBlock;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const poll = async () => {
    if (stopped) return;

    try {
      const latest = await provider.getBlockNumber();
      if (latest > lastBlock) {
        let filter;

        if (eventName === EVENT_MESSAGE_DISPATCHED) {
          filter = contract.filters.MessageDispatched();
        } else if (eventName === EVENT_MESSAGE_DELIVERED) {
          filter = contract.filters.MessageDelivered();
        } else {
          throw new Error(`Unsupported event name: ${eventName}`);
        }

        const events = await contract.queryFilter(filter, lastBlock + 1, latest);

        for (const event of events) {
          const log = event as ethers.EventLog;
          const msg: DispatchedMessage = {
            messageId: String(log.args[0]),
          };
          await onMessage(msg);
        }

        lastBlock = latest;
      }
    } catch (err) {
      console.error(`${LOG_DAPP} poll error:`, err);
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