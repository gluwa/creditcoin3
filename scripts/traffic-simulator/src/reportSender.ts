/**
 * Report sender script for Kubernetes CronJob
 *
 * Queries the traffic simulator's /status endpoint and sends periodic reports to Slack.
 * This script is designed to be run as a Kubernetes CronJob that queries metrics
 * and calculates deltas from the previous run.
 *
 * Configuration (environment variables):
 *   SIMULATOR_URL      - URL of the traffic simulator (default: http://traffic-simulator:8080)
 *   SLACK_WEBHOOK_URL  - Slack webhook URL (required)
 *   SLACK_ALERT_GROUP  - Slack user/group ID for alerts (optional)
 *   SNAPSHOT_PATH      - Path to store metrics snapshot (default: /tmp/metrics-snapshot.json)
 *   REPORT_INTERVAL_HOURS - Expected interval between reports in hours (default: 1)
 *   ALERT_SUCCESS_THRESHOLD_PCT - Success rate % below which alerts are triggered (default: 75)
 */

import {
  type HourlyReport,
  type MetricsSnapshot,
  sendHourlyReport,
  type SlackConfig,
} from "./slack.ts";

/**
 * Snapshot with timestamp for accurate period tracking
 */
interface TimestampedSnapshot {
  timestamp: number;
  metrics: MetricsSnapshot;
}

/**
 * Fetch current metrics from simulator
 */
async function fetchMetrics(
  simulatorUrl: string,
): Promise<MetricsSnapshot> {
  const url = new URL("/status", simulatorUrl);
  const response = await fetch(url.toString());

  if (!response.ok) {
    throw new Error(
      `Failed to fetch metrics: ${response.status} ${response.statusText}`,
    );
  }

  const data = await response.json();

  return {
    proofsSubmitted: data.proofsSubmitted ?? 0,
    proofErrors: data.proofErrors ?? 0,
    blocksProcessed: data.blocksProcessed ?? 0,
    singleSubmissions: data.singleSubmissions ?? 0,
    batchSubmissions: data.batchSubmissions ?? 0,
    queueSize: data.queueSize ?? 0,
    sepoliaConnected: data.sepoliaConnected ?? false,
    cc3Connected: data.cc3Connected ?? false,
    sourceChainKey: data.sourceChainKey ?? 1, // default to sepolia
    cc3WsUrl: data.cc3WsUrl ?? "",
    uptimeSeconds: data.uptimeSeconds ?? 0,
    lastError: data.lastError ?? null,
    uniqueErrors: data.uniqueErrors ?? {},
  };
}

/**
 * Load previous metrics snapshot from file
 */
async function loadPreviousSnapshot(
  snapshotPath: string,
): Promise<TimestampedSnapshot | null> {
  try {
    const content = await Deno.readTextFile(snapshotPath);
    const data = JSON.parse(content);

    // Check if it's the new timestamped format
    if (data.timestamp && data.metrics) {
      return data as TimestampedSnapshot;
    }

    // Legacy format: assume it's a raw MetricsSnapshot
    // Use file modification time as fallback timestamp
    const fileInfo = await Deno.stat(snapshotPath);
    const timestamp = fileInfo.mtime?.getTime() ?? Date.now();
    return {
      timestamp,
      metrics: data as MetricsSnapshot,
    };
  } catch {
    return null;
  }
}

/**
 * Save current metrics snapshot to file with timestamp
 */
async function saveSnapshot(
  snapshotPath: string,
  metrics: MetricsSnapshot,
  timestamp: number,
): Promise<void> {
  const snapshot: TimestampedSnapshot = {
    timestamp,
    metrics,
  };
  await Deno.writeTextFile(snapshotPath, JSON.stringify(snapshot, null, 2));
}

/**
 * Calculate delta between two counter values, handling restart detection.
 * If the delta is negative (counter reset due to pod restart), use the
 * current value as the delta since it represents activity since restart.
 */
function calculateDelta(previous: number, current: number): number {
  const delta = current - previous;
  return delta < 0 ? current : delta;
}

/**
 * Calculate hourly report from two snapshots
 */
function calculateReport(
  startMetrics: MetricsSnapshot,
  endMetrics: MetricsSnapshot,
  periodStart: number,
  periodEnd: number,
): HourlyReport {
  return {
    periodStart,
    periodEnd,
    startMetrics,
    endMetrics,
    delta: {
      proofsSubmitted: calculateDelta(
        startMetrics.proofsSubmitted,
        endMetrics.proofsSubmitted,
      ),
      proofErrors: calculateDelta(
        startMetrics.proofErrors,
        endMetrics.proofErrors,
      ),
      blocksProcessed: calculateDelta(
        startMetrics.blocksProcessed,
        endMetrics.blocksProcessed,
      ),
      singleSubmissions: calculateDelta(
        startMetrics.singleSubmissions,
        endMetrics.singleSubmissions,
      ),
      batchSubmissions: calculateDelta(
        startMetrics.batchSubmissions,
        endMetrics.batchSubmissions,
      ),
    },
  };
}

/**
 * Main function
 */
async function main(): Promise<void> {
  const simulatorUrl = Deno.env.get("SIMULATOR_URL") ||
    "http://traffic-simulator:8080";
  const slackWebhookUrl = Deno.env.get("SLACK_WEBHOOK_URL");
  const slackAlertGroup = Deno.env.get("SLACK_ALERT_GROUP");
  const slackBotToken = Deno.env.get("SLACK_BOT_TOKEN");
  const slackChannelId = Deno.env.get("SLACK_CHANNEL_ID");
  const snapshotPath = Deno.env.get("SNAPSHOT_PATH") ||
    "/tmp/metrics-snapshot.json";
  const reportIntervalHours = parseFloat(
    Deno.env.get("REPORT_INTERVAL_HOURS") || "1",
  );
  const alertSuccessThresholdPct = parseFloat(
    Deno.env.get("ALERT_SUCCESS_THRESHOLD_PCT") || "75",
  );

  if (!slackWebhookUrl && !slackBotToken) {
    console.error(
      "❌ Either SLACK_WEBHOOK_URL or SLACK_BOT_TOKEN environment variable is required",
    );
    Deno.exit(1);
  }

  if (slackBotToken && !slackChannelId) {
    console.error(
      "❌ SLACK_CHANNEL_ID is required when using SLACK_BOT_TOKEN",
    );
    Deno.exit(1);
  }

  if (isNaN(reportIntervalHours) || reportIntervalHours <= 0) {
    console.error(
      "❌ REPORT_INTERVAL_HOURS must be a positive number (got: " +
        Deno.env.get("REPORT_INTERVAL_HOURS") + ")",
    );
    Deno.exit(1);
  }

  if (
    isNaN(alertSuccessThresholdPct) || alertSuccessThresholdPct < 0 ||
    alertSuccessThresholdPct > 100
  ) {
    console.error(
      "❌ ALERT_SUCCESS_THRESHOLD_PCT must be a number between 0 and 100 (got: " +
        Deno.env.get("ALERT_SUCCESS_THRESHOLD_PCT") + ")",
    );
    Deno.exit(1);
  }

  const slackConfig: SlackConfig = {
    webhookUrl: slackWebhookUrl ?? "",
    alertGroup: slackAlertGroup,
    username: "traffic-simulator-reporter",
    alertSuccessThresholdPct,
    botToken: slackBotToken,
    channelId: slackChannelId,
  };

  try {
    console.log(`📊 Fetching metrics from ${simulatorUrl}...`);
    const currentMetrics = await fetchMetrics(simulatorUrl);
    console.log("✅ Metrics fetched successfully");

    const now = Date.now();
    const previousSnapshot = await loadPreviousSnapshot(snapshotPath);

    if (previousSnapshot) {
      const periodStart = previousSnapshot.timestamp;
      const actualHours = (now - periodStart) / 3600000;
      console.log(
        `📈 Calculating report for ${actualHours.toFixed(2)} hour period ` +
          `(configured interval: ${reportIntervalHours}h)...`,
      );
      const report = calculateReport(
        previousSnapshot.metrics,
        currentMetrics,
        periodStart,
        now,
      );

      await sendHourlyReport(report, slackConfig);
      console.log("✅ Report sent to Slack");

      // Reset unique errors so the next report only shows new errors
      try {
        const resetUrl = new URL("/reset-errors", simulatorUrl);
        const resetResponse = await fetch(resetUrl.toString(), {
          method: "POST",
        });
        if (resetResponse.ok) {
          console.log("🔄 Unique errors reset for next reporting period");
        } else {
          console.warn(
            `⚠️  Failed to reset errors: ${resetResponse.status} ${resetResponse.statusText}`,
          );
        }
      } catch (resetError) {
        console.warn("⚠️  Failed to reset errors:", resetError);
      }
    } else {
      console.log(
        `ℹ️  No previous snapshot found, skipping report ` +
          `(will start after ${reportIntervalHours}h)`,
      );
    }

    // Save current snapshot for next run
    await saveSnapshot(snapshotPath, currentMetrics, now);
    console.log(`💾 Snapshot saved to ${snapshotPath}`);
  } catch (error) {
    console.error("❌ Error:", error);
    Deno.exit(1);
  }
}

// Run if executed directly
if (import.meta.main) {
  main();
}
