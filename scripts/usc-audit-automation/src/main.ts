/**
 * USC Audit Automation
 *
 * Runs attestation sanity checks on USC (Creditcoin3) and reports to Slack or stdout.
 */

import { loadConfig } from "./config.ts";
import {
  connect,
  DEFAULT_MATURITY_STRATEGY,
  disconnect,
  getAttestationByDigest,
  getAttestationInterval,
  getCheckpointInterval,
  getLastCheckpoint,
  getLastDigest,
  getMaturityDelay,
  getSupportedChains,
  setVerbose,
} from "./usc.ts";
import {
  checkRpcHealthy,
  getBlockNumber,
  getBlockNumberByHash,
} from "./eth.ts";
import { queryAttestation } from "./graphql.ts";
import {
  BuiltReport,
  createSlackPayloads,
  sendSummarySlackMessage,
  sendThreadSlackMessage,
} from "./slack.ts";
import { runBalanceChecks } from "./balances.ts";

const BSC_MAX_BLOCK_DIFF = 499;
const ATTESTATION_LAG_BUFFER_INTERVALS = 3;

function formatNum(n: number): string {
  return n.toLocaleString("en-US");
}

function getMaxBlockDiff(
  chainName: string,
  maturityStrategy: string,
  attestationInterval: number,
): number {
  const formulaMax = getMaturityDelay(maturityStrategy) +
    attestationInterval * ATTESTATION_LAG_BUFFER_INTERVALS;
  return chainName.includes("BSC")
    ? Math.max(formulaMax, BSC_MAX_BLOCK_DIFF)
    : formulaMax;
}

function buildReport(
  chainLabel: string,
  chainId: number,
  chainKey: number,
  maturityStrategy: string,
  ethBlock: number,
  attBlock: number,
  maxBlockDiff: number,
  checkpointBlock: number,
  blockByHash: number | null,
  blockDiffOk: boolean,
  headerHashOk: boolean,
  checkpointRangeOk: boolean,
  graphqlAtt: { headerNumber: string; root: string; digest: string } | null,
  graphqlCp: { lastCheckpointHeaderNumber: string } | null,
): BuiltReport {
  const details: string[] = [];
  const title =
    `🚦 Attestation Chain Liveness: ${chainLabel} - ${chainId} - ${chainKey}`;
  const detailsTitle =
    `🚦 Liveness Details: ${chainLabel} - ${chainId} - ${chainKey} - ${maturityStrategy}`;

  details.push(
    (blockDiffOk ? "✅" : "❌") +
      ` Attestation block heights diff: ${formatNum(ethBlock - attBlock)} (${
        formatNum(ethBlock)
      }|${formatNum(attBlock)}|${formatNum(maxBlockDiff)})`,
  );

  const headerHashMatch = headerHashOk && blockByHash != null &&
    blockByHash === attBlock;

  details.push(
    (headerHashOk ? "✅" : "❌") +
      ` Attestation header hash matches correct Ethereum block${
        headerHashMatch
          ? ""
          : `: (${blockByHash != null ? formatNum(blockByHash) : "null"}|${
            formatNum(attBlock)
          })`
      }`,
  );

  details.push(
    (checkpointRangeOk ? "✅" : "❌") +
      ` Last checkpoint creation is ${
        checkpointRangeOk ? "within" : "outside"
      } checkpoint range${
        checkpointRangeOk
          ? ""
          : `: (${formatNum(ethBlock)}|${formatNum(checkpointBlock)})`
      }`,
  );

  if (graphqlAtt && graphqlCp) {
    const fmt = (s: string) => formatNum(Number(s) || 0);

    const cpMatch =
      fmt(graphqlCp.lastCheckpointHeaderNumber) === formatNum(checkpointBlock);
    const attMatch = fmt(graphqlAtt.headerNumber) === formatNum(attBlock);
    const hasRoot = graphqlAtt.root && /^0x[0-9a-fA-F]+$/.test(graphqlAtt.root);
    const hasDigest = graphqlAtt.digest &&
      /^0x[0-9a-fA-F]+$/.test(graphqlAtt.digest);

    details.push(
      (cpMatch ? "✅" : "❌") +
        ` Last checkpoint number found in GraphQL${
          cpMatch
            ? ""
            : `: (${fmt(graphqlCp.lastCheckpointHeaderNumber)}|${
              formatNum(checkpointBlock)
            })`
        }`,
    );

    details.push(
      (attMatch ? "✅" : "❌") +
        ` Last attestation header number found in GraphQL${
          attMatch
            ? ""
            : `: (${fmt(graphqlAtt.headerNumber)}|${formatNum(attBlock)})`
        }`,
    );

    details.push(
      (hasRoot ? "✅" : "❌") +
        ` Last attestation root found in GraphQL${
          hasRoot ? "" : `: (${graphqlAtt.root || "empty"})`
        }`,
    );

    details.push(
      (hasDigest ? "✅" : "❌") +
        ` Last attestation digest found in GraphQL${
          hasDigest ? "" : `: (${graphqlAtt.digest || "empty"})`
        }`,
    );
  } else {
    details.push("❌ GraphQL data not found for attestation/checkpoint");
  }

  const ok = details.every((line) => line.startsWith("✅"));
  const summary = `${title}\n${
    ok
      ? "✅ All liveness checks passed"
      : "❌ One or more liveness checks failed"
  }`;

  return {
    ok,
    summary,
    details: `${detailsTitle}\n${details.join("\n")}`,
  };
}

async function runChecksForChain(
  config: Awaited<ReturnType<typeof loadConfig>>,
  chainId: number,
  chainKey: number,
  chainName: string,
  ethRpcUrl: string,
  maturityStrategy: string,
): Promise<BuiltReport> {
  const chainLabel = `${chainName}`;
  const title = `🚦 Attestation chain liveness: ${chainLabel} - ${chainId}`;

  const lastDigest = await getLastDigest(chainKey);
  if (!lastDigest) {
    return {
      ok: false,
      summary:
        `${title}\n❌ No last digest for chain key ${chainKey}. Skipping.`,
      details: "",
    } satisfies BuiltReport;
  }

  const attestation = await getAttestationByDigest(chainKey, lastDigest);
  if (!attestation) {
    return {
      ok: false,
      summary:
        `${title}\n❌ Could not fetch attestation for lastDigest: ${lastDigest}. Skipping.`,
      details: "",
    } satisfies BuiltReport;
  }

  const lastCheckpoint = await getLastCheckpoint(chainKey);
  if (!lastCheckpoint) {
    return {
      ok: false,
      summary:
        `${title}\n❌ No last checkpoint for chain key ${chainKey}. Skipping.`,
      details: "",
    };
  }

  const ethBlock = await getBlockNumber(ethRpcUrl);
  const attBlock = attestation.headerNumber;

  const headerHashBytes = attestation.headerHash;
  const blockHash = "0x" +
    (headerHashBytes.length === 64
      ? headerHashBytes
      : headerHashBytes.padStart(64, "0"));
  const fetchedBlockByHash = await getBlockNumberByHash(ethRpcUrl, blockHash);
  const headerHashOk = fetchedBlockByHash === attBlock;

  const checkpointInterval = await getCheckpointInterval(chainKey);
  const attestationInterval = await getAttestationInterval(chainKey);
  const checkpointWidth = checkpointInterval * attestationInterval;
  const maxAttBlockDiff = getMaxBlockDiff(
    chainName,
    maturityStrategy,
    attestationInterval,
  );
  // maxCheckpointBlockDiff = maxAttBlockDiff + (lag bettween attestation creation and checkpoint creation)
  // Checkpoints are created when total_width_of_stored_attestations >= (checkpoint_width * 2) + 1
  const gapFromAttToCheck = checkpointWidth * 2 + 1;
  const maxCheckpointBlockDiff = maxAttBlockDiff + gapFromAttToCheck;
  const diff = ethBlock - lastCheckpoint.blockNumber;
  const checkpointRangeOk = diff >= 0 && diff <= maxCheckpointBlockDiff;

  if (config.verbose && !checkpointRangeOk) {
    console.log(
      `[${chainLabel}] Checkpoint range: diff=${diff}, maxAllowed=${maxCheckpointBlockDiff}`,
    );
  }

  const blockDiff = ethBlock - attBlock;
  const blockDiffOk = blockDiff >= 0 && blockDiff <= maxAttBlockDiff;

  const graphqlResult = await queryAttestation(
    config.graphqlUrl,
    chainKey,
    attBlock,
    lastCheckpoint.blockNumber,
  );

  const report = buildReport(
    chainLabel,
    chainId,
    chainKey,
    maturityStrategy,
    ethBlock,
    attBlock,
    maxAttBlockDiff,
    lastCheckpoint.blockNumber,
    fetchedBlockByHash,
    blockDiffOk,
    headerHashOk,
    checkpointRangeOk,
    graphqlResult.attestation,
    graphqlResult.checkpoint,
  );

  return report;
}

async function main(): Promise<void> {
  console.log("🛡️  USC Audit Automation");
  console.log("========================\n");

  const config = loadConfig();

  if (config.verbose) {
    console.log("Config:", {
      ...config,
      slackBotToken: config.slackBotToken ? "[REDACTED]" : undefined,
    });
  }

  setVerbose(config.verbose);
  await connect(config.uscWsUrl);
  console.log(`✅ Connected to USC at ${config.uscWsUrl}\n`);

  const supportedChains = await getSupportedChains();
  if (config.verbose && supportedChains.length > 0) {
    console.log(
      "Supported chains from USC:",
      supportedChains.map((c) => ({
        chainId: c.chainId,
        chainKey: c.chainKey,
        name: c.chainName,
        maturityStrategy: c.maturityStrategy,
      })),
    );
  }

  const reports: BuiltReport[] = [];
  const title = `🚦 Attestation chain liveness`;

  for (const ethRpc of config.ethRpc) {
    const healthy = await checkRpcHealthy(ethRpc.url);
    if (!healthy) {
      console.warn(`⚠️  RPC unhealthy for chain ${ethRpc.chainId}, skipping`);
      const report = {
        ok: false,
        summary:
          `${title} [Chain ${ethRpc.chainId}] ❌ RPC unhealthy - skipped`,
        details: "",
      } satisfies BuiltReport;
      reports.push(report);
      continue;
    }

    const discovered = supportedChains.find((c) =>
      c.chainId === ethRpc.chainId &&
      (ethRpc.chainKey == null || c.chainKey === ethRpc.chainKey)
    );
    const chainKey = discovered?.chainKey ?? ethRpc.chainKey;
    if (chainKey == null) {
      console.warn(
        `⚠️  No chain_key for chain ${ethRpc.chainId}, add chainKey to config`,
      );
      const report = {
        ok: false,
        summary:
          `${title} [Chain ${ethRpc.chainId}] ❌ No chain_key - add to config`,
        details: "",
      } satisfies BuiltReport;
      reports.push(report);
      continue;
    }

    const chainName = ethRpc.chainName ?? getChainName(ethRpc.chainId);
    const maturityStrategy = discovered?.maturityStrategy ??
      DEFAULT_MATURITY_STRATEGY;
    const report = await runChecksForChain(
      config,
      ethRpc.chainId,
      chainKey,
      chainName,
      ethRpc.url,
      maturityStrategy,
    );
    reports.push(report);
  }

  if (reports.length === 0) {
    if (supportedChains.length === 0) {
      const report = {
        ok: false,
        summary:
          `${title}: No supported chains found. Add ethRpc with chainKey to config.`,
        details: "",
      } satisfies BuiltReport;
      reports.push(report);
    }
  }

  // Add balances check report
  if (config.balanceChecks && config.balanceChecks.length > 0) {
    const balanceReport = await runBalanceChecks(config);
    reports.push(balanceReport);
  }

  for (const report of reports) {
    console.log("\n" + JSON.stringify(report, null, 2));
  }

  if (!config.noSlack && config.slackBotToken && config.slackChannelId) {
    const combinedSummaryReport = buildCombinedSummaryReport(
      config.uscNetworkName,
      reports,
    );

    const { summaryPayload } = createSlackPayloads(
      combinedSummaryReport,
      config.slackAlertGroup,
    );

    const thread_ts = await sendSummarySlackMessage(
      config.slackBotToken,
      config.slackChannelId,
      summaryPayload,
    );

    for (const report of reports) {
      if (!report.details || report.details.trim() === "") {
        continue;
      }

      const { detailsPayload } = createSlackPayloads(
        report,
        config.slackAlertGroup,
      );

      if (detailsPayload.text !== "") {
        await sendThreadSlackMessage(
          config.slackBotToken,
          config.slackChannelId,
          thread_ts,
          detailsPayload,
        );
      }
    }

    console.log(
      "\n✅ Combined summary sent to Slack, details posted in thread",
    );
  } else if (config.noSlack) {
    console.log("\n📋 Slack disabled (--no-slack); reports printed above");
  }

  disconnect();
}

function getChainName(chainId: number): string {
  const names: Record<number, string> = {
    11155111: "Sepolia",
    97: "BSC Testnet",
    1: "Ethereum Mainnet",
  };
  return names[chainId] ?? `Chain ${chainId}`;
}

function buildCombinedSummaryReport(
  networkName: string,
  reports: BuiltReport[],
): BuiltReport {
  const ok = reports.every((r) => r.ok);
  const title = `🛡️ USC Audit Summary [${networkName}]\n`;

  const summaryLines = reports.map((r) => r.summary);

  return {
    ok,
    summary: `${title}\n${summaryLines.join("\n\n")}`,
    details: "",
  };
}

main().catch((err) => {
  console.error("❌ Fatal error:", err);
  disconnect();
  Deno.exit(1);
});
