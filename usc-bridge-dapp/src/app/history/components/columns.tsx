"use client";

import Link from "next/link";
import type { ColumnDef } from "@tanstack/react-table";
import { formatEther } from "viem";

import { Badge } from "@/components/ui/badge";
import { HashChip } from "@/app/components/hash-chip";
import { spokeByChainKey } from "@/lib/chains";
import type { TransferRow } from "@/hooks/use-bridge-history";

export const columns: ColumnDef<TransferRow>[] = [
  {
    id: "route",
    header: "Route",
    cell: ({ row }) => {
      const source = spokeByChainKey(row.original.sourceChainKey);
      const dest = spokeByChainKey(row.original.destChainKey);
      return (
        <span className="whitespace-nowrap text-sm">
          {source?.chain.name ?? row.original.sourceChainKey} →{" "}
          {dest?.chain.name ?? row.original.destChainKey}
        </span>
      );
    },
  },
  {
    id: "amount",
    header: "Amount",
    cell: ({ row }) => {
      const dest = spokeByChainKey(row.original.destChainKey);
      return (
        <span>
          {formatEther(row.original.amount)}{" "}
          {dest?.chain.nativeCurrency.symbol ?? ""}
        </span>
      );
    },
  },
  {
    id: "depositTx",
    header: "Deposit",
    cell: ({ row }) => {
      const source = spokeByChainKey(row.original.sourceChainKey);
      return (
        <HashChip hash={row.original.depositTxHash} chain={source?.chain} />
      );
    },
  },
  {
    id: "status",
    header: "Status",
    cell: ({ row }) =>
      row.original.status === "released" ? (
        <Badge variant="success">Released</Badge>
      ) : (
        <Badge variant="secondary">Pending</Badge>
      ),
  },
  {
    id: "actions",
    header: "",
    cell: ({ row }) => (
      <Link
        href={`/progress/${row.original.depositTxHash}`}
        className="text-sm text-primary underline-offset-4 hover:underline"
      >
        Details
      </Link>
    ),
  },
];
