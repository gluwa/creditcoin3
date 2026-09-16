// localStorage index of transfers this browser has initiated or looked up — a convenience for
// Deposit's redirect and History's optimistic paint, never the source of truth. Every field here is
// re-derivable from chain state (see use-transfer-status.ts / use-bridge-history.ts), so losing
// this (private window, cleared storage, different browser) only costs a slower History load, not
// correctness.
import { create } from "zustand";
import { persist } from "zustand/middleware";

export interface KnownTransfer {
  depositTxHash: `0x${string}`;
  sourceChainKey: number;
  destChainKey: number;
  depositor: `0x${string}`;
  recipient: `0x${string}`;
  createdAt: number;
}

interface TransferStoreState {
  transfers: KnownTransfer[];
  addTransfer: (transfer: KnownTransfer) => void;
}

export const useTransferStore = create<TransferStoreState>()(
  persist(
    (set) => ({
      transfers: [],
      addTransfer: (transfer) =>
        set((state) => ({
          transfers: [
            transfer,
            ...state.transfers.filter(
              (t) => t.depositTxHash !== transfer.depositTxHash,
            ),
          ],
        })),
    }),
    { name: "usc-bridge-dapp:transfers" },
  ),
);
