"use client";

import Link from "next/link";
import { formatEther } from "viem";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Metric } from "@/app/components/metric";
import { HashChip } from "@/app/components/hash-chip";
import { StepIndicator } from "@/app/components/step-indicator";
import { spokeByChainKey } from "@/lib/chains";
import { useTransferStatus } from "@/hooks/use-transfer-status";

export function ProgressView({
  depositTxHash,
}: {
  depositTxHash: `0x${string}`;
}) {
  const status = useTransferStatus(depositTxHash);

  if (status.step === "loading") {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Looking up deposit...</CardTitle>
        </CardHeader>
      </Card>
    );
  }

  if (status.step === "not-found") {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Deposit not found</CardTitle>
        </CardHeader>
        <CardContent className="text-sm text-muted-foreground">
          <p>
            No <code>Deposited</code> event was found for{" "}
            <HashChip hash={depositTxHash} /> on any configured spoke chain. If
            you just submitted this deposit, it may still be confirming — reload
            in a few seconds.
          </p>
        </CardContent>
      </Card>
    );
  }

  const { deposit } = status;
  const destSpoke = deposit ? spokeByChainKey(deposit.destChainKey) : undefined;
  const title =
    status.step === "released"
      ? "Bridging complete"
      : status.step === "failed"
        ? "Bridging failed"
        : "Bridging in progress";

  return (
    <Card>
      <CardHeader>
        <CardTitle
          className={status.step === "failed" ? "text-destructive" : undefined}
        >
          {title}
        </CardTitle>
      </CardHeader>
      <CardContent className="flex flex-col gap-6">
        <StepIndicator status={status} />

        <div>
          <Metric
            label="Deposit tx"
            value={<HashChip hash={depositTxHash} />}
          />
          {deposit && (
            <>
              <Metric
                label="Amount"
                value={`${formatEther(deposit.amount)} ${destSpoke?.chain.nativeCurrency.symbol ?? ""}`}
              />
              <Metric
                label="Recipient"
                value={
                  <HashChip
                    hash={deposit.recipient}
                    kind="address"
                    chain={destSpoke?.chain}
                  />
                }
              />
              <Metric
                label="Destination"
                value={
                  destSpoke?.chain.name ?? `chain key ${deposit.destChainKey}`
                }
              />
            </>
          )}
          {status.releasedTxHash && (
            <Metric
              label="Release tx"
              value={
                <HashChip
                  hash={status.releasedTxHash}
                  chain={destSpoke?.chain}
                />
              }
            />
          )}
        </div>

        {status.step === "failed" ? (
          <p className="text-sm text-destructive">
            Delivery to the destination vault failed and will not be retried
            automatically — see the reason under &quot;Delivery failed&quot;
            above. Your deposit is not lost, but reaching the destination chain
            now needs manual intervention. Contact the bridge operator with this
            page&apos;s URL.
          </p>
        ) : (
          status.step !== "released" && (
            <p className="text-sm text-muted-foreground">
              This can take a few minutes. Bridging never loses a successful
              deposit — worst case it takes longer than expected, it will not
              fail silently. This page is safe to close and reload.
            </p>
          )
        )}
      </CardContent>
      <CardFooter className="gap-2">
        <Button asChild variant="outline">
          <Link href="/deposit">New deposit</Link>
        </Button>
        <Button asChild variant="ghost">
          <Link href="/history">View history</Link>
        </Button>
      </CardFooter>
    </Card>
  );
}
