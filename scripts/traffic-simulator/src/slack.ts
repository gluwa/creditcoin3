/**
 * Slack notification utilities for traffic simulator
 *
 * Sends formatted reports to Slack via webhook.
 */

export interface SlackConfig {
  /** Slack webhook URL (used when botToken/channelId are not set) */
  webhookUrl: string;
  /** Optional Slack user/group ID to mention in alerts (e.g., "U123456" or "S123456") */
  alertGroup?: string;
  /** Username for Slack messages */
  username?: string;
  /** Success rate threshold percentage below which alerts are triggered (default: 75) */
  alertSuccessThresholdPct?: number;
  /** Slack Bot Token for API-based messaging (enables thread replies) */
  botToken?: string;
  /** Slack Channel ID for API-based messaging (required with botToken) */
  channelId?: string;
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
  /** Unique errors with occurrence counts */
  uniqueErrors: Record<string, number>;
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

/** Slack section block text limit */
const SLACK_SECTION_TEXT_MAX = 3000;

function truncateForSlack(text: string, max = SLACK_SECTION_TEXT_MAX): string {
  if (text.length <= max) return text;
  const suffix = "\n… (truncated)";
  return text.slice(0, max - suffix.length) + suffix;
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
 * Create Slack payload for hourly report using a single code block
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

  const padLeft = (str: string, width: number): string => {
    return String(str).padStart(width);
  };

  // Pad label column - use fixed width padding
  const padLabel = (str: string, width: number): string => {
    return String(str).padEnd(width);
  };

  // Build report text in code block format with tables
  const reportLines: string[] = [
    `${headerEmoji} Traffic Simulator ${periodLabel} Report`,
    "",
    `🕐 Period: ${periodStartStr} → ${periodEndStr} UTC (${
      periodHours.toFixed(1)
    }h)`,
    "",
    "🔗 Chains & Connection",
    "┌─────────┬─────────────────────────────┐",
    `│ Source  │ ${
      padLabel(
        (endMetrics.sepoliaConnected ? "🟢" : "🔴") + " " + sourceChain,
        27,
      )
    } │`,
    `├─────────┼─────────────────────────────┤`,
    `│ Target  │ ${
      padLabel(
        (endMetrics.cc3Connected ? "🟢" : "🔴") + " " + targetNetwork,
        27,
      )
    } │`,
    "└─────────┴─────────────────────────────┘",
    "",
    `📤 Proof Submissions (${periodHours.toFixed(1)}h)`,
    "┌──────────────────┬──────────────┐",
    `│ ${padLabel("✅ Successful", 15)} │ ${
      padLeft(formatNumber(delta.proofsSubmitted), 12)
    } │`,
    `├──────────────────┼──────────────┤`,
    `│ ${padLabel("❌ Failed", 15)} │ ${
      padLeft(formatNumber(delta.proofErrors), 12)
    } │`,
    `├──────────────────┼──────────────┤`,
    `│ ${padLabel("📈 Rate", 16)} │ ${padLeft(proofsPerHour + "/hr", 12)} │`,
    `├──────────────────┼──────────────┤`,
    `│ ${padLabel("🎯 Success", 16)} │ ${padLeft(successRate + "%", 12)} │`,
    "└──────────────────┴──────────────┘",
    "",
    `📋 Breakdown & Blocks (${periodHours.toFixed(1)}h)`,
    "┌──────────────────┬──────────────┬──────────────────┬──────────────┐",
    `│ ${padLabel("📝 Single", 16)} │ ${
      padLeft(formatNumber(delta.singleSubmissions), 12)
    } │ ${padLabel("📦 Batch", 16)} │ ${
      padLeft(formatNumber(delta.batchSubmissions), 12)
    } │`,
    `├──────────────────┼──────────────┼──────────────────┼──────────────┤`,
    `│ ${padLabel("⚙️  Processed", 16)} │ ${
      padLeft(formatNumber(delta.blocksProcessed), 12)
    } │ ${padLabel("📋 Queue", 16)} │ ${
      padLeft(formatNumber(endMetrics.queueSize), 12)
    } │`,
    "└──────────────────┴──────────────┴──────────────────┴──────────────┘",
    "",
    "📊 Totals",
    "┌──────────────────┬──────────────┬──────────────────┬──────────────┐",
    `│ ${padLabel("✅ Proofs", 15)} │ ${
      padLeft(formatNumber(endMetrics.proofsSubmitted), 12)
    } │ ${padLabel("❌ Errors", 15)} │ ${
      padLeft(formatNumber(endMetrics.proofErrors), 12)
    } │`,
    `├──────────────────┼──────────────┼──────────────────┼──────────────┤`,
    `│ ${padLabel("📦 Blocks", 16)} │ ${
      padLeft(formatNumber(endMetrics.blocksProcessed), 12)
    } │ ${padLabel("⏱️  Uptime", 16)} │ ${
      padLeft(formatUptime(endMetrics.uptimeSeconds), 12)
    } │`,
    "└──────────────────┴──────────────┴──────────────────┴──────────────┘",
  ];

  if (endMetrics.lastError) {
    const maxErrorLen = 256;
    const truncated = endMetrics.lastError.length > maxErrorLen
      ? endMetrics.lastError.slice(0, maxErrorLen) + "…"
      : endMetrics.lastError;
    reportLines.push("");
    reportLines.push(`🚨 Last Error: ${truncated}`);
  }

  const reportText = reportLines.join("\n");

  // Build text for notifications (fallback)
  let text = `Traffic Simulator ${periodLabel} Report: ${
    formatNumber(delta.proofsSubmitted)
  } proofs submitted`;
  if (hasErrors) {
    text += `, ${formatNumber(delta.proofErrors)} errors`;
  }

  const totalAttempts = delta.proofsSubmitted + delta.proofErrors;

  const successRatePct = totalAttempts > 0
    ? (delta.proofsSubmitted / totalAttempts) * 100
    : 100;

  const alertSuccessThresholdPct = config.alertSuccessThresholdPct ?? 75;

  const shouldAlertTeam = Boolean(config.alertGroup) &&
    totalAttempts > 0 &&
    successRatePct < alertSuccessThresholdPct;

  let mentionPrefix = "";
  if (shouldAlertTeam && config.alertGroup) {
    try {
      mentionPrefix = `${formatSlackMention(config.alertGroup)} `;
    } catch (error) {
      console.warn(`Failed to format Slack mention: ${error}`);
    }
  }

  return {
    username: config.username || "traffic-simulator",
    icon_emoji: hasErrors ? ":rotating_light:" : ":chart_with_upwards_trend:",
    link_names: true,
    text,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: truncateForSlack(`\`\`\`${reportText}\`\`\``),
        },
      },
      ...(mentionPrefix
        ? [
          { type: "divider" },
          {
            type: "section",
            text: {
              type: "mrkdwn",
              text:
                `🚨 Errors detected, proof submission success rate below threshold (success ${successRate}% < ${alertSuccessThresholdPct}%), tagging ${mentionPrefix.trim()}`,
            },
          },
        ]
        : []),
    ],
  };
}

/**
 * Send message to Slack via API (chat.postMessage) or webhook fallback.
 * Returns the message timestamp when using the API (needed for thread replies).
 */
export async function sendSlackMessage(
  config: SlackConfig,
  payload: unknown,
  threadTs?: string,
): Promise<string | undefined> {
  // Use Slack API when bot token and channel are configured
  if (config.botToken && config.channelId) {
    const apiPayload = {
      channel: config.channelId,
      ...(payload as Record<string, unknown>),
      ...(threadTs ? { thread_ts: threadTs } : {}),
    };

    const response = await fetch("https://slack.com/api/chat.postMessage", {
      method: "POST",
      headers: {
        "Content-Type": "application/json; charset=utf-8",
        "Authorization": `Bearer ${config.botToken}`,
      },
      body: JSON.stringify(apiPayload),
    });

    if (!response.ok) {
      const text = await response.text();
      throw new Error(
        `Slack API request failed: ${response.status} ${response.statusText} - ${text}`,
      );
    }

    const result = await response.json();
    if (!result.ok) {
      throw new Error(`Slack API error: ${result.error}`);
    }

    return result.ts as string;
  }

  // Fallback to webhook (no thread support)
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

  return undefined;
}

/**
 * Send hourly report to Slack, with the latest error as a thread reply
 * when using the Slack API (botToken + channelId configured).
 */
export async function sendHourlyReport(
  report: HourlyReport,
  config: SlackConfig,
): Promise<void> {
  const payload = createHourlyReportPayload(report, config);
  const messageTs = await sendSlackMessage(config, payload);

  // Send errors as a thread reply when using the Slack API
  const errors = report.endMetrics.uniqueErrors;
  const errorEntries = Object.entries(errors);
  const hasUniqueErrors = errorEntries.length > 0;
  const hasLastError = Boolean(report.endMetrics.lastError);

  if (messageTs && (hasUniqueErrors || hasLastError)) {
    let errorText: string;
    let errorSummary: string;

    if (hasUniqueErrors) {
      const errorLines = errorEntries.map(
        ([msg, count]) => `• (x${count}) ${msg}`,
      );
      errorText = errorLines.join("\n");
      errorSummary = `${errorEntries.length} Unique Error(s)`;
    } else {
      errorText = `• ${report.endMetrics.lastError}`;
      errorSummary = "Latest Error";
    }

    const errorPayload = {
      username: config.username || "traffic-simulator",
      icon_emoji: ":rotating_light:",
      text: errorSummary,
      blocks: [
        {
          type: "section",
          text: {
            type: "mrkdwn",
            text: truncateForSlack(
              `:rotating_light: *${errorSummary}*\n\`\`\`${errorText}\`\`\``,
            ),
          },
        },
      ],
    };

    try {
      await sendSlackMessage(config, errorPayload, messageTs);
    } catch (error) {
      console.warn(`Failed to send error thread reply: ${error}`);
    }
  }
}
