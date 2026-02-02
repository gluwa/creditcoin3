/**
 * Slack notification utilities for traffic simulator
 *
 * Sends formatted reports to Slack via webhook.
 */

export interface SlackConfig {
  /** Slack webhook URL */
  webhookUrl: string;
  /** Optional Slack user/group ID to mention in alerts (e.g., "U123456" or "S123456") */
  alertGroup?: string;
  /** Username for Slack messages */
  username?: string;
}

export interface MetricsSnapshot {
  /** Total proofs submitted successfully */
  proofsSubmitted: number;
  /** Total proof submission errors */
  proofErrors: number;
  /** Total blocks processed */
  blocksProcessed: number;
  /** Total single submissions */
  singleSubmissions: number;
  /** Total batch submissions */
  batchSubmissions: number;
  /** Current queue size */
  queueSize: number;
  /** Whether connected to Sepolia */
  sepoliaConnected: boolean;
  /** Whether connected to CC3 */
  cc3Connected: boolean;
  /** Source chain key (e.g., 1 for Sepolia) */
  sourceChainKey: number;
  /** CC3 WebSocket URL */
  cc3WsUrl: string;
  /** Uptime in seconds */
  uptimeSeconds: number;
  /** Last error message if any */
  lastError: string | null;
}

export interface HourlyReport {
  /** Report period start timestamp */
  periodStart: number;
  /** Report period end timestamp */
  periodEnd: number;
  /** Metrics at start of period */
  startMetrics: MetricsSnapshot;
  /** Metrics at end of period */
  endMetrics: MetricsSnapshot;
  /** Delta calculations */
  delta: {
    proofsSubmitted: number;
    proofErrors: number;
    blocksProcessed: number;
    singleSubmissions: number;
    batchSubmissions: number;
  };
}

/**
 * Format Slack user/group mention
 */
function formatSlackMention(id: string): string {
  if (id.startsWith("U")) {
    return `<@${id}>`;
  } else if (id.startsWith("S")) {
    return `<!subteam^${id}>`;
  }
  throw new Error(`Unexpected Slack ID format: ${id}`);
}

/**
 * Get source chain name from chain key
 */
function getSourceChainName(chainKey: number): string {
  const chainNames: Record<number, string> = {
    1: "Sepolia",
    2: "Ethereum Mainnet",
  };
  return chainNames[chainKey] ?? `Chain ${chainKey}`;
}

/**
 * Get target network name from CC3 WebSocket URL
 */
function getTargetNetworkName(cc3WsUrl: string): string {
  const url = cc3WsUrl.toLowerCase();
  if (url.includes("devnet") || url.includes("dev")) {
    return "USC Devnet";
  } else if (url.includes("testnet") || url.includes("test")) {
    return "USC Testnet";
  } else if (url.includes("mainnet") || url.includes("main")) {
    return "Creditcoin Mainnet";
  } else if (url.includes("localhost") || url.includes("127.0.0.1")) {
    return "Local";
  }
  return "Creditcoin";
}

/**
 * Format number with thousand separators
 */
function formatNumber(num: number): string {
  return num.toLocaleString("en-US");
}

/**
 * Format uptime in human-readable format
 */
function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  const secs = seconds % 60;

  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  if (minutes > 0) parts.push(`${minutes}m`);
  if (secs > 0 || parts.length === 0) parts.push(`${secs}s`);

  return parts.join(" ");
}

/**
 * Format period label based on duration in hours
 */
function formatPeriodLabel(hours: number): string {
  if (hours < 1.5) {
    return "Hourly";
  } else if (hours < 3) {
    return "2-Hour";
  } else if (hours < 5) {
    return "4-Hour";
  } else if (hours < 9) {
    return "6-Hour";
  } else if (hours < 18) {
    return "12-Hour";
  } else if (hours < 36) {
    return "Daily";
  } else {
    return `${Math.round(hours)}-Hour`;
  }
}

/**
 * Create Slack payload for hourly report using Block Kit
 */
export function createHourlyReportPayload(
  report: HourlyReport,
  config: SlackConfig,
): unknown {
  const { delta, endMetrics, periodStart, periodEnd } = report;
  const periodDuration = periodEnd - periodStart;
  const periodHours = periodDuration / 3600000;

  const successRate = delta.proofsSubmitted + delta.proofErrors > 0
    ? (
      (delta.proofsSubmitted /
        (delta.proofsSubmitted + delta.proofErrors)) *
      100
    ).toFixed(1)
    : "N/A";

  const proofsPerHour = periodHours > 0
    ? (delta.proofsSubmitted / periodHours).toFixed(1)
    : "0";

  const periodStartStr = new Date(periodStart).toISOString().replace("T", " ")
    .slice(0, 19);
  const periodEndStr = new Date(periodEnd).toISOString().replace("T", " ")
    .slice(0, 19);

  const allConnected = endMetrics.sepoliaConnected && endMetrics.cc3Connected;
  const hasErrors = delta.proofErrors > 0;

  const periodLabel = formatPeriodLabel(periodHours);
  const sourceChain = getSourceChainName(endMetrics.sourceChainKey);
  const targetNetwork = getTargetNetworkName(endMetrics.cc3WsUrl);

  // Header emoji based on status
  const headerEmoji = hasErrors ? "🚨" : allConnected ? "📊" : "⚠️";

  // Build blocks array
  const blocks: unknown[] = [
    // Header
    {
      type: "header",
      text: {
        type: "plain_text",
        text: `${headerEmoji} Traffic Simulator ${periodLabel} Report`,
        emoji: true,
      },
    },
    // Period info
    {
      type: "context",
      elements: [
        {
          type: "mrkdwn",
          text: `🕐 *${periodStartStr}* → *${periodEndStr}* UTC  •  ${
            periodHours.toFixed(1)
          }h`,
        },
      ],
    },
    { type: "divider" },
    // Chains & Connection Status
    {
      type: "section",
      text: {
        type: "mrkdwn",
        text: "*🔗 Chains & Connection*",
      },
    },
    {
      type: "section",
      fields: [
        {
          type: "mrkdwn",
          text: `*Source*\n${
            endMetrics.sepoliaConnected ? "🟢" : "🔴"
          } ${sourceChain}`,
        },
        {
          type: "mrkdwn",
          text: `*Target*\n${
            endMetrics.cc3Connected ? "🟢" : "🔴"
          } ${targetNetwork}`,
        },
      ],
    },
    { type: "divider" },
    // Proof Submissions
    {
      type: "section",
      text: {
        type: "mrkdwn",
        text: "*📤 Proof Submissions*",
      },
    },
    {
      type: "section",
      fields: [
        {
          type: "mrkdwn",
          text: `*Successful*\n✅ ${formatNumber(delta.proofsSubmitted)}`,
        },
        {
          type: "mrkdwn",
          text: `*Failed*\n${hasErrors ? "❌" : "✅"} ${
            formatNumber(delta.proofErrors)
          }`,
        },
        {
          type: "mrkdwn",
          text: `*Rate*\n📈 ${proofsPerHour}/hr`,
        },
        {
          type: "mrkdwn",
          text: `*Success*\n🎯 ${successRate}%`,
        },
      ],
    },
    { type: "divider" },
    // Breakdown & Blocks
    {
      type: "section",
      fields: [
        {
          type: "mrkdwn",
          text: `*📋 Breakdown*\nSingle: ${
            formatNumber(delta.singleSubmissions)
          }\nBatch: ${formatNumber(delta.batchSubmissions)}`,
        },
        {
          type: "mrkdwn",
          text: `*📦 Blocks*\nProcessed: ${
            formatNumber(delta.blocksProcessed)
          }\nQueue: ${formatNumber(endMetrics.queueSize)}`,
        },
      ],
    },
    { type: "divider" },
    // Totals
    {
      type: "context",
      elements: [
        {
          type: "mrkdwn",
          text: `📊 *Totals:* ${
            formatNumber(endMetrics.proofsSubmitted)
          } proofs  •  ${formatNumber(endMetrics.proofErrors)} errors  •  ${
            formatNumber(endMetrics.blocksProcessed)
          } blocks  •  ⏱️ ${formatUptime(endMetrics.uptimeSeconds)}`,
        },
      ],
    },
  ];

  // Add error section if there's a last error
  if (endMetrics.lastError) {
    blocks.push(
      { type: "divider" },
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: `*🚨 Last Error*\n\`\`\`${endMetrics.lastError}\`\`\``,
        },
      },
    );
  }

  // Build text for notifications (fallback)
  let text = `Traffic Simulator ${periodLabel} Report: ${
    formatNumber(delta.proofsSubmitted)
  } proofs submitted`;
  if (hasErrors) {
    text += `, ${formatNumber(delta.proofErrors)} errors`;
  }

  // Add alert mention if there are errors and alert group is configured
  if (hasErrors && config.alertGroup) {
    try {
      const mention = formatSlackMention(config.alertGroup);
      text = `${mention} ${text}`;
    } catch (error) {
      console.warn(`Failed to format Slack mention: ${error}`);
    }
  }

  return {
    username: config.username || "traffic-simulator",
    icon_emoji: hasErrors ? ":rotating_light:" : ":chart_with_upwards_trend:",
    text,
    blocks,
  };
}

/**
 * Send message to Slack via webhook
 */
export async function sendSlackMessage(
  config: SlackConfig,
  payload: unknown,
): Promise<void> {
  const response = await fetch(config.webhookUrl, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify(payload),
  });

  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      `Slack webhook failed: ${response.status} ${response.statusText} - ${text}`,
    );
  }
}

/**
 * Send hourly report to Slack
 */
export async function sendHourlyReport(
  report: HourlyReport,
  config: SlackConfig,
): Promise<void> {
  const payload = createHourlyReportPayload(report, config);
  await sendSlackMessage(config, payload);
}
