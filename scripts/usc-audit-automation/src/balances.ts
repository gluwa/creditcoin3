/**
 * Balance audits for USC audit reports
 */

import type { BuiltReport } from "./slack.ts";
import type { AuditConfig } from "./config.ts";

export interface BalanceAccountConfig {
  address: string;
  name?: string;
}

export interface BalanceNetworkConfig {
  name: string;
  baseUrl: string;
  rpcUrl?: string;
  accounts: BalanceAccountConfig[];
}

const MAX_RETRIES = 3;
const RETRY_DELAY_MS = 2000;
const TOKEN_SYMBOL = "CTC";
const TOKEN_DECIMALS = 18;
const THRESHOLD_CTC = 10;
const THRESHOLD_WEI = BigInt(THRESHOLD_CTC) * (10n ** BigInt(TOKEN_DECIMALS));

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

function formatTokenFromWei(wei: bigint): string {
  return (Number(wei) / 10 ** TOKEN_DECIMALS).toFixed(6);
}

function formatDisplay(account: BalanceAccountConfig): string {
  return account.name
    ? `${account.address} (${account.name})`
    : account.address;
}

async function fetchJsonWithRetry<T>(
  input: RequestInfo | URL,
  init?: RequestInit,
): Promise<T> {
  let lastError: unknown;

  for (let attempt = 0; attempt < MAX_RETRIES; attempt++) {
    try {
      const res = await fetch(input, init);
      if (!res.ok) {
        throw new Error(`HTTP ${res.status} ${res.statusText}`);
      }
      return await res.json() as T;
    } catch (err) {
      lastError = err;
      if (attempt < MAX_RETRIES - 1) {
        await sleep(RETRY_DELAY_MS);
      }
    }
  }

  throw new Error(
    `request failed after ${MAX_RETRIES} attempts: ${
      lastError instanceof Error ? lastError.message : String(lastError)
    }`,
  );
}

export async function fetchBalanceBlockscout(
  baseUrl: string,
  address: string,
): Promise<bigint> {
  const url = new URL("/api", baseUrl);
  url.searchParams.set("module", "account");
  url.searchParams.set("action", "balance");
  url.searchParams.set("address", address);

  const data = await fetchJsonWithRetry<{ result?: unknown }>(url);

  if (typeof data.result === "string" && /^\d+$/.test(data.result)) {
    return BigInt(data.result);
  }

  throw new Error(`unexpected response: ${JSON.stringify(data)}`);
}

export async function fetchBalanceRpc(
  rpcUrl: string,
  address: string,
): Promise<bigint> {
  const payload = {
    jsonrpc: "2.0",
    method: "eth_getBalance",
    params: [address, "latest"],
    id: 1,
  };

  const data = await fetchJsonWithRetry<{ result?: unknown; error?: unknown }>(
    rpcUrl,
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    },
  );

  if (data.error) {
    throw new Error(`RPC error: ${JSON.stringify(data.error)}`);
  }

  if (
    typeof data.result === "string" &&
    /^0x[0-9a-fA-F]+$/.test(data.result)
  ) {
    return BigInt(data.result);
  }

  throw new Error(`unexpected RPC response: ${JSON.stringify(data)}`);
}

export async function fetchBalance(
  baseUrl: string,
  address: string,
  rpcUrl?: string,
): Promise<bigint> {
  try {
    const bal = await fetchBalanceBlockscout(baseUrl, address);

    if (bal === 0n && rpcUrl) {
      try {
        const rpcBal = await fetchBalanceRpc(rpcUrl, address);
        if (rpcBal > 0n) {
          console.log(
            `Blockscout returned 0 for ${address} but RPC reports ${
              formatTokenFromWei(rpcBal)
            } ${TOKEN_SYMBOL}, using RPC value`,
          );
          return rpcBal;
        }
      } catch (rpcErr) {
        console.warn(
          `RPC fallback failed for ${address}: ${
            rpcErr instanceof Error ? rpcErr.message : String(rpcErr)
          }`,
        );
      }
    }

    return bal;
  } catch (blockscoutErr) {
    if (!rpcUrl) {
      throw blockscoutErr;
    }

    console.warn(
      `Blockscout failed for ${address}, falling back to RPC: ${
        blockscoutErr instanceof Error
          ? blockscoutErr.message
          : String(blockscoutErr)
      }`,
    );

    return await fetchBalanceRpc(rpcUrl, address);
  }
}

export async function runBalanceChecks(
  config: AuditConfig,
): Promise<BuiltReport> {
  const networks = config.balanceChecks ?? [];

  if (networks.length === 0) {
    return {
      ok: true,
      summary: "💸 USC balance audit\n✅ No balance checks configured",
      details: "",
    };
  }

  const lines: string[] = [];
  const lowLines: string[] = [];
  let hasErrors = false;
  let hasLowBalances = false;

  for (const net of networks) {
    lines.push(`Balances Details: ${net.name}`);

    if (!net.baseUrl) {
      hasErrors = true;
      lines.push("❌ missing baseUrl");
      lines.push("");
      continue;
    }

    if (!net.accounts?.length) {
      hasErrors = true;
      lines.push("❌ no accounts configured");
      lines.push("");
      continue;
    }

    for (const account of net.accounts) {
      const display = formatDisplay(account);

      try {
        const bal = await fetchBalance(
          net.baseUrl,
          account.address,
          net.rpcUrl,
        );
        const token = Number(bal) / 10 ** TOKEN_DECIMALS;
        const isLow = bal < THRESHOLD_WEI;

        lines.push(
          `${isLow ? "❌" : "✅"} ${display}: ${
            token.toFixed(6)
          } ${TOKEN_SYMBOL}`,
        );

        if (isLow) {
          hasLowBalances = true;
          lowLines.push(
            `- ${config.uscNetworkName}, \`${display}\`: ${
              token.toFixed(6)
            } ${TOKEN_SYMBOL}`,
          );
        }
      } catch (err) {
        hasErrors = true;
        lines.push(
          `❌ ${display}: ${err instanceof Error ? err.message : String(err)}`,
        );
      }
    }

    lines.push("");
  }

  if (lowLines.length > 0) {
    lines.push("*Low balance alert*");
    lines.push(`Threshold: ${THRESHOLD_CTC} ${TOKEN_SYMBOL}`);
    lines.push(...lowLines);
  }

  const ok = !hasErrors && !hasLowBalances;
  const title = `💸 USC Balance Audit:`;

  let mention = "";

  if (hasLowBalances && config.slackAlertGroup) {
    mention = config.slackAlertGroup.startsWith("S")
      ? `<!subteam^${config.slackAlertGroup}>`
      : config.slackAlertGroup.startsWith("U")
      ? `<@${config.slackAlertGroup}>`
      : config.slackAlertGroup;
  }

  const summary = ok
    ? `${title}\n✅ All monitored balances are healthy`
    : hasLowBalances
    ? `${title}\n❌ One or more monitored balances are below threshold\n${mention}`
    : `${title}\n❌ One or more balance checks failed`;

  return {
    ok,
    summary,
    details: lines.join("\n").trim(),
  };
}
