// Polls one spoke chain's BridgeVault for `Deposited` events, holding back `confirmationDepth`
// blocks from the tip. Mirrors dApp-ack-worker/listeners.ts's poll-and-advance-cursor shape.
import { ethers } from "ethers";

const BRIDGE_VAULT_ABI = [
  "event Deposited(uint256 indexed nonce, address indexed depositor, address indexed recipient, uint32 destChainKey, address token, uint256 amount)",
];

export interface DepositEvent {
  chainKey: number;
  nonce: bigint;
  depositor: string;
  recipient: string;
  destChainKey: number;
  token: string;
  amount: bigint;
  txHash: string;
  blockNumber: number;
}

export type StopFn = () => void;

export function watchDeposits(
  provider: ethers.JsonRpcProvider,
  chainKey: number,
  vaultAddress: string,
  fromBlock: number,
  confirmationDepth: number,
  pollIntervalMs: number,
  onDeposit: (event: DepositEvent) => void | Promise<void>,
): StopFn {
  const contract = new ethers.Contract(
    vaultAddress,
    BRIDGE_VAULT_ABI,
    provider,
  );

  let lastScanned = fromBlock;
  let stopped = false;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const poll = async () => {
    if (stopped) return;

    try {
      const tip = await provider.getBlockNumber();
      const safeHead = tip - confirmationDepth;

      if (safeHead > lastScanned) {
        const filter = contract.filters.Deposited();
        const logs = await contract.queryFilter(
          filter,
          lastScanned + 1,
          safeHead,
        );

        for (const log of logs) {
          const event = log as ethers.EventLog;
          await onDeposit({
            chainKey,
            nonce: event.args[0] as bigint,
            depositor: event.args[1] as string,
            recipient: event.args[2] as string,
            destChainKey: Number(event.args[3]),
            token: event.args[4] as string,
            amount: event.args[5] as bigint,
            txHash: event.transactionHash,
            blockNumber: event.blockNumber,
          });
        }

        lastScanned = safeHead;
      }
    } catch (err) {
      console.error(`[watcher chainKey=${chainKey}] poll error:`, err);
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
