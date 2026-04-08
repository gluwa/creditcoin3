/**
 * Slack notification utilities for traffic simulator
 *
 * Sends formatted reports to Slack via webhook.
 */

import type { HealthStatus, SlackPayload } from "../types.ts";

// The default success percentage for proof submission below which we alert the dev team on slack.
const DEFAULT_PROOF_SUCCESS_ALERT_THRESHOLD = 75;
// The default success percentage for proof submission below which we print
// a warning message in the traffic report summary.
const DEFAULT_PROOF_SUCCESS_WARNING_THRESHOLD = 95;
// The rate measured in proofs/hour below which we alert the dev team
const DEFAULT_EXPECTED_PROOFS_PER_HOUR = 10;

export interface SlackConfig {
  /** Slack webhook URL (used when botToken/channelId are not set) */
  webhookUrl: string;
  /** Optional Slack user/group ID to mention in alerts (e.g., "U123456" or "S123456") */
  alertGroup?: string;
  /** Username for Slack messages */
  username?: string;
  /** Success rate threshold percentage below which alerts are triggered (default: 75) */
  alertSuccessThresholdPct?: number;
  /** Success rate threshold percentage below which a warning is printed in the summary */
  warningSuccessThresholdPct?: number;
  /** Proof submission volume per hour below which alerts are triggered (default: 10/hour) */
  proofVolumeAlertThreshold?: number;
  /** Slack Bot Token for API-based messaging (enables thread replies) */
  botToken?: string;
  /** Slack Channel ID for API-based messaging (required with botToken) */
  channelId?: string;
}

export interface PeriodicReport {
  /** Report period start timestamp */
  periodStart: number;
  /** Report period end timestamp */
  periodEnd: number;
  /** Status snapshot at start of period */
  startSnapshot: HealthStatus;
  /** Status snapshot at end of period */
  endSnapshot: HealthStatus;
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
 * Create Slack payloads for report summary and report body
 */
export function createReportPayloads(
  report: PeriodicReport,
  config: SlackConfig,
): { reportSummary: SlackPayload; reportBody: SlackPayload } {
  const { delta, endSnapshot, periodStart, periodEnd } = report;
  const periodDuration = periodEnd - periodStart;
  const periodHours = periodDuration / 3600000;

  const successRate = delta.proofsSubmitted + delta.proofErrors > 0
    ? (
      (delta.proofsSubmitted /
        (delta.proofsSubmitted + delta.proofErrors)) *
      100
    ).toFixed(1)
    : "N/A";

  const proofsPerHourNum = periodHours > 0
    ? delta.proofsSubmitted / periodHours
    : 0;

  const proofsPerHour = proofsPerHourNum.toFixed(1);

  const periodStartStr = new Date(periodStart).toISOString().replace("T", " ")
    .slice(0, 19);
  const periodEndStr = new Date(periodEnd).toISOString().replace("T", " ")
    .slice(0, 19);

  const allConnected = endSnapshot.sourceChainConnected &&
    endSnapshot.cc3Connected;

  const periodLabel = formatPeriodLabel(periodHours);
  const sourceChain = getSourceChainName(endSnapshot.sourceChainKey);
  const targetNetwork = getTargetNetworkName(endSnapshot.cc3WsUrl);

  const totalAttempts = delta.proofsSubmitted + delta.proofErrors;

  // Determing any warnings and alerts to trigger
  const successRatePct = totalAttempts > 0
    ? (delta.proofsSubmitted / totalAttempts) * 100
    : 100;

  const alertSuccessThresholdPct = config.alertSuccessThresholdPct ?? DEFAULT_PROOF_SUCCESS_ALERT_THRESHOLD;
  const warningSuccessThresholdPct = config.warningSuccessThresholdPct ?? DEFAULT_PROOF_SUCCESS_WARNING_THRESHOLD;
  const proofVolumeAlertThreshold = config.proofVolumeAlertThreshold ?? DEFAULT_EXPECTED_PROOFS_PER_HOUR;

  const triggerSuccessThresholdAlert = Boolean(config.alertGroup) &&
    totalAttempts > 0 &&
    successRatePct < alertSuccessThresholdPct;
  const triggerSuccessThresholdWarning = triggerSuccessThresholdAlert
    ? false
    : totalAttempts > 0 &&
      successRatePct < warningSuccessThresholdPct;
  const triggerProofVolumeAlert = Boolean(config.alertGroup) &&
    totalAttempts > 0 && (proofsPerHourNum < proofVolumeAlertThreshold);

  const alertOrWarning = triggerSuccessThresholdAlert ||
    triggerSuccessThresholdWarning || triggerProofVolumeAlert;
  // Header emoji based on status
  const headerEmoji = allConnected ? "📊" : "⚠️";

  // Build report summary
  const reportSummaryLines: string[] = [
    `${headerEmoji} Traffic Simulator Report`,
    `🕐 Period: ${periodStartStr} → ${periodEndStr} UTC (${
      periodHours.toFixed(1)
    }h)`,
    `✅ Proofs submitted: ${formatNumber(delta.proofsSubmitted)}`,
    `❌ Proofs Failed: ${formatNumber(delta.proofErrors)}`,
  ];
  if (triggerSuccessThresholdWarning) {
    reportSummaryLines.push(
      `⚠️⚠️⚠️ Proof success rate below warning threshold: ${successRatePct} < ${warningSuccessThresholdPct} ⚠️⚠️⚠️`,
    );
  }

  const summaryText = reportSummaryLines.join("\n");

  // Build report body with tables
  const reportBodyLines: string[] = [
    "🔗 *Chains & Connection*",
    `• Source: ${(endSnapshot.sourceChainConnected
      ? "🟢"
      : "🔴")} ${sourceChain}`,
    `• Target: ${(endSnapshot.cc3Connected ? "🟢" : "🔴")} ${targetNetwork}`,
    "",
    `📤 *Proof Submissions (${periodHours.toFixed(1)}h)*`,
    `• ✅ Successful: ${formatNumber(delta.proofsSubmitted)}`,
    `• ❌ Failed: ${formatNumber(delta.proofErrors)}`,
    `• 📈 Rate: ${proofsPerHour}/hr`,
    `• 🎯 Success: ${successRate}%`,
    "",
    `📋 *Breakdown & Blocks (${periodHours.toFixed(1)}h)*`,
    `• 📝 Single: ${formatNumber(delta.singleSubmissions)}`,
    `• 📦 Batch: ${formatNumber(delta.batchSubmissions)}`,
    `• ⚙️ Processed: ${formatNumber(delta.blocksProcessed)}`,
    `• 📋 Queue: ${formatNumber(endSnapshot.queueSize)}`,
    "",
    "*📊 Totals Since Startup*",
    `• ✅ Proofs: ${formatNumber(endSnapshot.proofsSubmitted)}`,
    `• ❌ Errors: ${formatNumber(endSnapshot.proofErrors)}`,
    `• 📦 Blocks: ${formatNumber(endSnapshot.blocksProcessed)}`,
    `• ⏱️ Uptime: ${formatUptime(endSnapshot.uptimeSeconds)}`,
  ];

  if (endSnapshot.lastError) {
    const maxErrorLen = 256;
    const truncated = endSnapshot.lastError.length > maxErrorLen
      ? endSnapshot.lastError.slice(0, maxErrorLen) + "…"
      : endSnapshot.lastError;
    reportBodyLines.push("");
    reportBodyLines.push(`🚨 Last Error: ${truncated}`);
  }

  const bodyText = reportBodyLines.join("\n");

  // Build text for notifications (fallback)
  let text = `Traffic Simulator ${periodLabel} Report: ${
    formatNumber(delta.proofsSubmitted)
  } proofs submitted`;
  if (alertOrWarning) {
    text += `, ${formatNumber(delta.proofErrors)} errors`;
  }

  // Build alert text using status from earlier
  let mentionPrefix = "";
  let alertText = "";
  if (
    config.alertGroup &&
    (triggerSuccessThresholdAlert || triggerProofVolumeAlert)
  ) {
    // Set mention prefix
    try {
      mentionPrefix = `${formatSlackMention(config.alertGroup)} `;
    } catch (error) {
      console.warn(`Failed to format Slack mention: ${error}`);
    }
    // Build alert text. Capture both alert conditions if both are present.
    const alertReasons: string[] = [];

    if (triggerSuccessThresholdAlert) {
      alertReasons.push(
        `proof submission success rate below threshold (success ${successRate}% < ${alertSuccessThresholdPct}%)`,
      );
    }

    if (triggerProofVolumeAlert) {
      alertReasons.push(
        `processed proof volume below threshold (${proofsPerHour} proofs/hour < ${proofVolumeAlertThreshold})`,
      );
    }

    alertText = `🚨 Alert(s) triggered, ${
      alertReasons.join(" and ")
    }, tagging ${mentionPrefix.trim()}`;
  }

  const reportSummary: SlackPayload = {
    username: config.username || "traffic-simulator",
    icon_emoji: alertOrWarning
      ? ":rotating_light:"
      : ":chart_with_upwards_trend:",
    link_names: true,
    text,
    blocks: [
      {
        type: "section" as const,
        text: {
          type: "mrkdwn" as const,
          text: truncateForSlack(`\`\`\`${summaryText}\`\`\``),
        },
      },
      ...(mentionPrefix
        ? [
            { type: "divider" as const },
            {
              type: "section" as const,
              text: {
                type: "mrkdwn" as const,
                text: alertText,
              },
            },
          ]
        : []),
    ],
  };

  const reportBody: SlackPayload = {
    username: config.username || "traffic-simulator",
    icon_emoji: alertOrWarning
      ? ":rotating_light:"
      : ":chart_with_upwards_trend:",
    link_names: true,
    text,
    blocks: [
      {
        type: "section",
        text: {
          type: "mrkdwn",
          text: truncateForSlack(`\`\`\`${bodyText}\`\`\``),
        },
      },
    ],
  };

  return { reportSummary, reportBody };
}

/**
 * Send message to Slack via API (chat.postMessage) or webhook fallback.
 * Returns the message timestamp when using the API (needed for thread replies).
 */
export async function sendSlackMessage(
  config: SlackConfig,
  payload: SlackPayload,
  threadTs?: string,
): Promise<string | undefined> {
  // Use Slack API when bot token and channel are configured
  if (config.botToken && config.channelId) {
    const apiPayload = {
      channel: config.channelId,
      ...payload,
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
 * Send periodic report to Slack, with the latest error as a thread reply
 * when using the Slack API (botToken + channelId configured).
 */
export async function sendPeriodicReport(
  report: PeriodicReport,
  config: SlackConfig,
): Promise<void> {
  const { reportSummary, reportBody } = createReportPayloads(report, config);
  const messageTs = await sendSlackMessage(config, reportSummary);

  // Send report body as thread reply
  try {
    await sendSlackMessage(config, reportBody, messageTs);
  } catch (error) {
    console.warn(`Failed to send report body thread reply: ${error}`);
  }

  // Send errors as a thread reply when using the Slack API
  const errors = report.endSnapshot.uniqueErrors;
  const errorEntries = Object.entries(errors);
  const hasUniqueErrors = errorEntries.length > 0;
  const hasLastError = Boolean(report.endSnapshot.lastError);

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
      errorText = `• ${report.endSnapshot.lastError}`;
      errorSummary = "Latest Error";
    }

    const errorPayload: SlackPayload = {
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
