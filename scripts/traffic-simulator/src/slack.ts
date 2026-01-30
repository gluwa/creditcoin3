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
 * Create Slack payload for hourly report
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

  const periodStartStr = new Date(periodStart).toISOString();
  const periodEndStr = new Date(periodEnd).toISOString();

  const statusEmoji = endMetrics.sepoliaConnected && endMetrics.cc3Connected
    ? "✅"
    : "⚠️";

  const errorEmoji = delta.proofErrors > 0 ? "❌" : "✅";

  const periodLabel = formatPeriodLabel(periodHours);
  let text = `📊 *Traffic Simulator ${periodLabel} Report*\n\n`;
  text += `*Period:* ${periodStartStr} → ${periodEndStr}\n`;
  text += `*Duration:* ${periodHours.toFixed(2)} hours\n\n`;

  text += `*Connection Status:* ${statusEmoji}\n`;
  text += `  • Sepolia: ${
    endMetrics.sepoliaConnected ? "✅ Connected" : "❌ Disconnected"
  }\n`;
  text += `  • CC3: ${
    endMetrics.cc3Connected ? "✅ Connected" : "❌ Disconnected"
  }\n\n`;

  text += `*Proof Submissions:*\n`;
  text += `  • Successful: ${
    formatNumber(delta.proofsSubmitted)
  } (${proofsPerHour}/hr)\n`;
  text += `  • Failed: ${errorEmoji} ${formatNumber(delta.proofErrors)}\n`;
  text += `  • Success Rate: ${successRate}%\n\n`;

  text += `*Submission Breakdown:*\n`;
  text += `  • Single: ${formatNumber(delta.singleSubmissions)}\n`;
  text += `  • Batch: ${formatNumber(delta.batchSubmissions)}\n\n`;

  text += `*Blocks:*\n`;
  text += `  • Processed: ${formatNumber(delta.blocksProcessed)}\n`;
  text += `  • Queue Size: ${formatNumber(endMetrics.queueSize)}\n\n`;

  text += `*Totals (since start):*\n`;
  text += `  • Proofs Submitted: ${formatNumber(endMetrics.proofsSubmitted)}\n`;
  text += `  • Proof Errors: ${formatNumber(endMetrics.proofErrors)}\n`;
  text += `  • Blocks Processed: ${formatNumber(endMetrics.blocksProcessed)}\n`;
  text += `  • Uptime: ${formatUptime(endMetrics.uptimeSeconds)}\n`;

  if (endMetrics.lastError) {
    text += `\n*Last Error:*\n\`\`\`${endMetrics.lastError}\`\`\``;
  }

  // Add alert mention if there are errors and alert group is configured
  if (delta.proofErrors > 0 && config.alertGroup) {
    try {
      const mention = formatSlackMention(config.alertGroup);
      text = `${mention} ${text}`;
    } catch (error) {
      console.warn(`Failed to format Slack mention: ${error}`);
    }
  }

  return {
    username: config.username || "traffic-simulator",
    icon_emoji: delta.proofErrors > 0
      ? ":rotating_light:"
      : ":chart_with_upwards_trend:",
    text,
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
