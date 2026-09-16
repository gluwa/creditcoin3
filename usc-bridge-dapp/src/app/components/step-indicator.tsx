import type { ReactNode } from "react";
import { Check, Loader2, X } from "lucide-react";

import { cn } from "@/lib/utils";
import { spokeByChainKey } from "@/lib/chains";
import { HashChip } from "./hash-chip";
import type { TransferStatus } from "@/hooks/use-transfer-status";

type StepState = "done" | "active" | "pending" | "unknown" | "failed";

interface StepDef {
  label: string;
  state: StepState;
  detail?: ReactNode;
}

function buildSteps(status: TransferStatus): StepDef[] {
  const isDeposited = status.step !== "loading" && status.step !== "not-found";
  const isAttested =
    status.step === "attested" ||
    status.step === "released" ||
    status.step === "failed";
  const isReleased = status.step === "released";
  const isFailed = status.step === "failed";

  const source =
    status.sourceChainKey !== undefined
      ? spokeByChainKey(status.sourceChainKey)
      : undefined;
  const destSpoke = status.deposit
    ? spokeByChainKey(status.deposit.destChainKey)
    : undefined;

  return [
    {
      label: "Deposited",
      state: isDeposited ? "done" : "active",
      detail: status.deposit ? (
        <>
          Block #{status.deposit.blockNumber.toString()} on{" "}
          {source?.chain.name ?? "the source chain"}
        </>
      ) : undefined,
    },
    {
      label: "Attested",
      state: isAttested ? "done" : isDeposited ? "active" : "pending",
      detail: isAttested
        ? status.attestedHeight != null
          ? `Attested at height ${status.attestedHeight}`
          : "Attested"
        : isDeposited
          ? "Waiting for the source block to be attested..."
          : undefined,
    },
    {
      label: "Claimed",
      state:
        status.claimed === undefined
          ? "unknown"
          : status.claimed
            ? "done"
            : isAttested
              ? "active"
              : "pending",
      detail:
        status.claimed === undefined ? undefined : status.claimed ? (
          status.claimTxHash ? (
            <>
              Claim tx <HashChip hash={status.claimTxHash} />
            </>
          ) : (
            "Claimed on Creditcoin"
          )
        ) : isAttested ? (
          "Waiting for the claimer bot to submit the claim..."
        ) : undefined,
    },
    {
      label: isFailed ? "Delivery failed" : "Released",
      state: isFailed
        ? "failed"
        : isReleased
          ? "done"
          : isAttested
            ? "active"
            : "pending",
      detail: isFailed ? (
        <>
          {status.failure?.reason}
          {status.failure?.txHash && (
            <>
              {" — "}
              <HashChip hash={status.failure.txHash} chain={destSpoke?.chain} />
            </>
          )}
        </>
      ) : isReleased && status.releasedTxHash ? (
        <>
          Release tx{" "}
          <HashChip hash={status.releasedTxHash} chain={destSpoke?.chain} />
        </>
      ) : isAttested ? (
        "Waiting for delivery to the destination chain..."
      ) : undefined,
    },
  ];
}

export function StepIndicator({ status }: { status: TransferStatus }) {
  const steps = buildSteps(status);

  return (
    <ol className="flex flex-col gap-4">
      {steps.map((step) => (
        <li key={step.label} className="flex items-start gap-3 text-sm">
          <span
            className={cn(
              "mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border text-xs",
              step.state === "done" &&
                "border-emerald-600 bg-emerald-600 text-white",
              step.state === "active" && "border-primary text-primary",
              step.state === "pending" &&
                "border-muted-foreground/30 text-muted-foreground",
              step.state === "unknown" &&
                "border-dashed border-muted-foreground/30 text-muted-foreground",
              step.state === "failed" &&
                "border-destructive bg-destructive text-destructive-foreground",
            )}
          >
            {step.state === "done" && <Check className="h-3.5 w-3.5" />}
            {step.state === "active" && (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            )}
            {step.state === "failed" && <X className="h-3.5 w-3.5" />}
          </span>
          <div className="flex flex-col">
            <span
              className={cn(
                step.state === "pending" || step.state === "unknown"
                  ? "text-muted-foreground"
                  : "font-medium",
                step.state === "failed" && "text-destructive",
              )}
            >
              {step.label}
              {step.state === "unknown" && (
                <span className="ml-1.5 text-xs font-normal text-muted-foreground">
                  (not tracked)
                </span>
              )}
            </span>
            {step.detail && (
              <span
                className={cn(
                  "text-xs text-muted-foreground",
                  step.state === "failed" && "text-destructive",
                )}
              >
                {step.detail}
              </span>
            )}
          </div>
        </li>
      ))}
    </ol>
  );
}
