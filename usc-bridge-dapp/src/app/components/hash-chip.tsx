"use client";

import { useState } from "react";
import { Check, Copy, ExternalLink } from "lucide-react";
import type { Chain } from "viem";

import { cn } from "@/lib/utils";

function truncate(hash: string): string {
  return `${hash.slice(0, 6)}…${hash.slice(-4)}`;
}

export function HashChip({
  hash,
  chain,
  kind = "tx",
  className,
}: {
  hash: string;
  chain?: Chain;
  kind?: "tx" | "address";
  className?: string;
}) {
  const [copied, setCopied] = useState(false);
  const explorerBase = chain?.blockExplorers?.default.url;
  const href = explorerBase
    ? `${explorerBase}/${kind === "tx" ? "tx" : "address"}/${hash}`
    : undefined;

  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 font-mono text-xs",
        className,
      )}
    >
      {truncate(hash)}
      <button
        type="button"
        aria-label="Copy"
        onClick={() => {
          void navigator.clipboard.writeText(hash);
          setCopied(true);
          setTimeout(() => setCopied(false), 1500);
        }}
        className="text-muted-foreground hover:text-foreground"
      >
        {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
      </button>
      {href && (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="text-muted-foreground hover:text-foreground"
        >
          <ExternalLink className="h-3 w-3" />
        </a>
      )}
    </span>
  );
}
