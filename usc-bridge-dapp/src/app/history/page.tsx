"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import { useAccount } from "wagmi";
import { isHash } from "viem";

import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { useBridgeHistory } from "@/hooks/use-bridge-history";
import { DataTable } from "./components/data-table";
import { columns } from "./components/columns";

export default function HistoryPage() {
  const { address } = useAccount();
  const { rows, isLoading } = useBridgeHistory(address);
  const router = useRouter();
  const [lookup, setLookup] = useState("");

  function onLookup(e: React.FormEvent) {
    e.preventDefault();
    if (isHash(lookup)) router.push(`/progress/${lookup}`);
  }

  return (
    <div className="flex flex-col gap-6">
      <Card>
        <CardHeader>
          <CardTitle>Look up a transfer</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={onLookup} className="flex gap-2">
            <Input
              placeholder="Deposit tx hash (0x...)"
              value={lookup}
              onChange={(e) => setLookup(e.target.value)}
            />
            <Button type="submit" disabled={!isHash(lookup)}>
              View
            </Button>
          </form>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>Your transfers</CardTitle>
        </CardHeader>
        <CardContent>
          {!address ? (
            <p className="text-sm text-muted-foreground">
              Connect a wallet to see your transfers.
            </p>
          ) : isLoading && rows.length === 0 ? (
            <p className="text-sm text-muted-foreground">
              Scanning chain history...
            </p>
          ) : (
            <DataTable columns={columns} data={rows} />
          )}
        </CardContent>
      </Card>
    </div>
  );
}
