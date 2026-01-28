/**
 * Report sender script for Kubernetes CronJob
 *
 * Queries the traffic simulator's /status endpoint and sends hourly reports to Slack.
 * This script is designed to be run as a Kubernetes CronJob that queries metrics
 * and calculates deltas from the previous run.
 */

import {
  type HourlyReport,
  type MetricsSnapshot,
  sendHourlyReport,
  type SlackConfig,
} from "./slack.ts";

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
): Promise<MetricsSnapshot | null> {
  try {
    const content = await Deno.readTextFile(snapshotPath);
    return JSON.parse(content) as MetricsSnapshot;
  } catch {
    return null;
  }
}

/**
 * Save current metrics snapshot to file
 */
async function saveSnapshot(
  snapshotPath: string,
  metrics: MetricsSnapshot,
): Promise<void> {
  await Deno.writeTextFile(snapshotPath, JSON.stringify(metrics, null, 2));
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
      proofsSubmitted: Math.max(0, endMetrics.proofsSubmitted - startMetrics.proofsSubmitted),
      proofErrors: Math.max(0, endMetrics.proofErrors - startMetrics.proofErrors),
      blocksProcessed: Math.max(0, endMetrics.blocksProcessed - startMetrics.blocksProcessed),
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
  const snapshotPath = Deno.env.get("SNAPSHOT_PATH") || "/tmp/metrics-snapshot.json";

  if (!slackWebhookUrl) {
    console.error("❌ SLACK_WEBHOOK_URL environment variable is required");
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
    const previousMetrics = await loadPreviousSnapshot(snapshotPath);

    if (previousMetrics) {
      console.log("📈 Calculating hourly report...");
      const report = calculateReport(previousMetrics, currentMetrics, now - 3600000, now);
      await sendHourlyReport(report, slackConfig);
      console.log("✅ Hourly report sent to Slack");
    } else {
      console.log("ℹ️  No previous snapshot found, skipping report (will start next hour)");
    }

    // Save current snapshot for next run
    await saveSnapshot(snapshotPath, currentMetrics);
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
