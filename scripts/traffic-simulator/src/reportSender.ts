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
    uptimeSeconds: data.uptimeSeconds ?? 0,
    lastError: data.lastError ?? null,
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
      proofsSubmitted: Math.max(
        0,
        endMetrics.proofsSubmitted - startMetrics.proofsSubmitted,
      ),
      proofErrors: Math.max(
        0,
        endMetrics.proofErrors - startMetrics.proofErrors,
      ),
      blocksProcessed: Math.max(
        0,
        endMetrics.blocksProcessed - startMetrics.blocksProcessed,
      ),
      singleSubmissions: Math.max(
        0,
        endMetrics.singleSubmissions - startMetrics.singleSubmissions,
      ),
      batchSubmissions: Math.max(
        0,
        endMetrics.batchSubmissions - startMetrics.batchSubmissions,
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
  const snapshotPath = Deno.env.get("SNAPSHOT_PATH") ||
    "/tmp/metrics-snapshot.json";
  const reportIntervalHours = parseFloat(
    Deno.env.get("REPORT_INTERVAL_HOURS") || "1",
  );

  if (!slackWebhookUrl) {
    console.error("❌ SLACK_WEBHOOK_URL environment variable is required");
    Deno.exit(1);
  }

  if (isNaN(reportIntervalHours) || reportIntervalHours <= 0) {
    console.error(
      "❌ REPORT_INTERVAL_HOURS must be a positive number (got: " +
        Deno.env.get("REPORT_INTERVAL_HOURS") + ")",
    );
    Deno.exit(1);
  }

  const slackConfig: SlackConfig = {
    webhookUrl: slackWebhookUrl,
    alertGroup: slackAlertGroup,
    username: "traffic-simulator-reporter",
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
