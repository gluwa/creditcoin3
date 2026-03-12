/**
 * USC Audit Automation
 *
 * Runs attestation sanity checks on USC (Creditcoin3) and reports to Slack or stdout.
 * Style matches traffic-simulator: Deno + TypeScript, CLI + env config.
 */

import { loadConfig } from "./config.ts";
import {
  connect,
  disconnect,
  getAttestationByDigest,
  getAttestationInterval,
  getCheckpointInterval,
  getLastCheckpoint,
  getLastDigest,
  getSupportedChains,
  setVerbose,
} from "./usc.ts";
import {
  checkRpcHealthy,
  getBlockNumber,
  getBlockNumberByHash,
} from "./eth.ts";
import { queryAttestation } from "./graphql.ts";
import { createSlackPayload, sendSlackMessage } from "./slack.ts";

const MAX_BLOCK_DIFF = 40;
const BSC_MAX_BLOCK_DIFF = 499;

function formatNum(n: number): string {
  return n.toLocaleString("en-US");
}

function getMaxBlockDiff(chainName: string): number {
  return chainName.includes("BSC") ? BSC_MAX_BLOCK_DIFF : MAX_BLOCK_DIFF;
}

function buildReport(
  chainLabel: string,
  chainId: number,
  ethBlock: number,
  attBlock: number,
  checkpointBlock: number,
  blockByHash: number | null,
  blockDiffOk: boolean,
  headerHashOk: boolean,
  checkpointRangeOk: boolean,
  graphqlAtt: { headerNumber: string; root: string; digest: string } | null,
  graphqlCp: { lastCheckpointHeaderNumber: string } | null,
): string {
  const lines: string[] = [];
  lines.push(`[${chainLabel} - ${chainId}] ⬛ ${chainLabel}`);
  lines.push(
    (blockDiffOk ? "✅" : "❌") +
      ` Attestation block heights diff: ${formatNum(ethBlock - attBlock)} (${
        formatNum(ethBlock)
      }|${formatNum(attBlock)})`,
  );
  const headerHashMatch = headerHashOk && blockByHash != null &&
    blockByHash === attBlock;
  lines.push(
    (headerHashOk ? "✅" : "❌") +
      ` Attestation header hash matches correct Ethereum block${
        headerHashMatch
          ? ""
          : `: (${blockByHash != null ? formatNum(blockByHash) : "null"}|${
            formatNum(attBlock)
          })`
      }`,
  );
  lines.push(
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
    lines.push(
      (cpMatch ? "✅" : "❌") +
        ` Last checkpoint number found in GraphQL${
          cpMatch
            ? ""
            : `: (${fmt(graphqlCp.lastCheckpointHeaderNumber)}|${
              formatNum(checkpointBlock)
            })`
        }`,
    );
    lines.push(
      (attMatch ? "✅" : "❌") +
        ` Last attestation header number found in GraphQL${
          attMatch
            ? ""
            : `: (${fmt(graphqlAtt.headerNumber)}|${formatNum(attBlock)})`
        }`,
    );
    lines.push(
      (hasRoot ? "✅" : "❌") +
        ` Last attestation root found in GraphQL${
          hasRoot ? "" : `: (${graphqlAtt.root || "empty"})`
        }`,
    );
    lines.push(
      (hasDigest ? "✅" : "❌") +
        ` Last attestation digest found in GraphQL${
          hasDigest ? "" : `: (${graphqlAtt.digest || "empty"})`
        }`,
    );
  } else {
    lines.push("❌ GraphQL data not found for attestation/checkpoint");
  }
  return lines.join("\n");
}

async function runChecksForChain(
  config: Awaited<ReturnType<typeof loadConfig>>,
  chainId: number,
  chainKey: number,
  chainName: string,
  ethRpcUrl: string,
): Promise<{ report: string; hasErrors: boolean }> {
  const chainLabel = `${config.uscNetworkName} - ${chainName}`;

  const lastDigest = await getLastDigest(chainKey);
  if (!lastDigest) {
    return {
      report:
        `[${chainLabel}] ❌ No last digest for chain key ${chainKey}. Skipping.`,
      hasErrors: true,
    };
  }

  const attestation = await getAttestationByDigest(chainKey, lastDigest);
  if (!attestation) {
    return {
      report:
        `[${chainLabel}] ❌ Could not fetch attestation for digest. Skipping.`,
      hasErrors: true,
    };
  }

  const lastCheckpoint = await getLastCheckpoint(chainKey);
  if (!lastCheckpoint) {
    return {
      report:
        `[${chainLabel}] ❌ No last checkpoint for chain key ${chainKey}. Skipping.`,
      hasErrors: true,
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
  // Pallet creates checkpoint when attestation_block_span >= (checkpoint_width * 2) + 1
  // Adds two attestation intervals as buffer for submission/timing variance
  const maxAllowed = checkpointWidth * 2 + 1 + (attestationInterval * 2);
  const diff = ethBlock - lastCheckpoint.blockNumber;
  const checkpointRangeOk = diff >= 0 && diff <= maxAllowed;

  if (config.verbose && !checkpointRangeOk) {
    console.log(
      `[${chainLabel}] Checkpoint range: diff=${diff}, maxAllowed=${maxAllowed} (checkpointWidth*2+1+attestationInterval=${checkpointWidth}*2+1+${attestationInterval})`,
    );
  }

  const blockDiff = ethBlock - attBlock;
  const maxBlockDiff = getMaxBlockDiff(chainName);
  const blockDiffOk = blockDiff >= 0 && blockDiff <= maxBlockDiff;

  const graphqlResult = await queryAttestation(
    config.graphqlUrl,
    chainKey,
    attBlock,
    lastCheckpoint.blockNumber,
  );

  const att = graphqlResult.attestation;
  const cp = graphqlResult.checkpoint;
  const graphqlCpMatch = att != null && cp != null &&
    cp.lastCheckpointHeaderNumber === String(lastCheckpoint.blockNumber);
  const graphqlAttMatch = att != null && att.headerNumber === String(attBlock);
  const graphqlHasRoot = att?.root != null && /^0x[0-9a-fA-F]+$/.test(att.root);
  const graphqlHasDigest = att?.digest != null &&
    /^0x[0-9a-fA-F]+$/.test(att.digest);

  const report = buildReport(
    chainLabel,
    chainId,
    ethBlock,
    attBlock,
    lastCheckpoint.blockNumber,
    fetchedBlockByHash,
    blockDiffOk,
    headerHashOk,
    checkpointRangeOk,
    graphqlResult.attestation,
    graphqlResult.checkpoint,
  );

  const hasErrors = !blockDiffOk || !headerHashOk || !checkpointRangeOk ||
    !graphqlCpMatch || !graphqlAttMatch || !graphqlHasRoot || !graphqlHasDigest;
  return { report, hasErrors };
}

async function main(): Promise<void> {
  console.log("🛡️  USC Audit Automation");
  console.log("========================\n");

  const config = loadConfig();

  if (config.verbose) {
    console.log("Config:", {
      ...config,
      slackWebhookUrl: config.slackWebhookUrl ? "[REDACTED]" : undefined,
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
      })),
    );
  }

  const reports: string[] = [];
  let anyErrors = false;

  for (const ethRpc of config.ethRpc) {
    const healthy = await checkRpcHealthy(ethRpc.url);
    if (!healthy) {
      console.warn(`⚠️  RPC unhealthy for chain ${ethRpc.chainId}, skipping`);
      reports.push(`[Chain ${ethRpc.chainId}] ❌ RPC unhealthy - skipped`);
      anyErrors = true;
      continue;
    }

    const discovered = supportedChains.find((c) =>
      c.chainId === ethRpc.chainId
    );
    const chainKey = discovered?.chainKey ?? ethRpc.chainKey;
    if (chainKey == null) {
      console.warn(
        `⚠️  No chain_key for chain ${ethRpc.chainId}, add chainKey to config`,
      );
      reports.push(
        `[Chain ${ethRpc.chainId}] ❌ No chain_key - add to config`,
      );
      anyErrors = true;
      continue;
    }

    const chainName = getChainName(ethRpc.chainId);
    const { report, hasErrors } = await runChecksForChain(
      config,
      ethRpc.chainId,
      chainKey,
      chainName,
      ethRpc.url,
    );
    reports.push(report);
    if (hasErrors) anyErrors = true;
  }

  if (reports.length === 0) {
    if (supportedChains.length === 0) {
      reports.push(
        "No supported chains found. Add ethRpc with chainKey to config.",
      );
    }
  }

  const fullReport = reports.join("\n\n");
  console.log("\n" + fullReport);

  if (!config.noSlack && config.slackWebhookUrl) {
    const payload = createSlackPayload(
      fullReport,
      anyErrors,
      config.slackAlertGroup,
    );
    await sendSlackMessage(config.slackWebhookUrl, payload);
    console.log("\n✅ Report sent to Slack");
  } else if (config.noSlack) {
    console.log("\n📋 Slack disabled (--no-slack); report printed above");
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

main().catch((err) => {
  console.error("❌ Fatal error:", err);
  disconnect();
  Deno.exit(1);
});
