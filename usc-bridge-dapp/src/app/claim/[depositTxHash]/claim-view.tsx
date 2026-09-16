"use client";

import Link from "next/link";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardFooter,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { StepIndicator } from "@/app/components/step-indicator";
import { HashChip } from "@/app/components/hash-chip";
import { useTransferStatus } from "@/hooks/use-transfer-status";

// Status-only, no manual "claim now" fallback: a fallback would need the user to hold a funded
// Creditcoin wallet and raw proof bytes — directly undermining the "invisible hub" goal — and a
// slow/down claimer bot is an operational problem (fix the bot), not a UX problem to solve by
// asking a test user to do a harder, riskier action. See the bridge POC plan §5.
export function ClaimView({ depositTxHash }: { depositTxHash: `0x${string}` }) {
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
          <CardDescription>
            No <code>Deposited</code> event was found for{" "}
            <HashChip hash={depositTxHash} /> on any configured spoke chain.
          </CardDescription>
        </CardHeader>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Claim status</CardTitle>
        <CardDescription>
          Claiming happens automatically — a standalone bot submits the proof to
          BridgeHub on Creditcoin as soon as your deposit is attested. There is
          nothing for you to sign here.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <StepIndicator status={status} />
      </CardContent>
      <CardFooter className="gap-2">
        <Button asChild variant="outline">
          <Link href={`/progress/${depositTxHash}`}>Full progress</Link>
        </Button>
      </CardFooter>
    </Card>
  );
}
