import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { Interface, Wallet, getAddress, id, toQuantity } from "ethers";
import type { Rpc, Scan } from "./rpc.mjs";
import { FatalError, json, readJson } from "./runtime.mjs";
import { ANVIL_KEY, SOURCE_KEY, type Stack } from "./stack.mjs";

const outbox = new Interface([
  "event MessagePublished(bytes32 indexed messageId,bytes32 indexed emitterAddress,uint64 sequence,bool canAck,bytes payload)",
  "event MessageAcknowledged(bytes32 indexed messageId)",
  "function isAcknowledged(bytes32) view returns(bool)",
]);
const inbox = new Interface([
  "function processedAt(bytes32) view returns(uint256)",
  "event MessageExecuted(bytes32 indexed messageId,address indexed emitterAddress,address indexed destination,address dispatcher,address relayer,bytes messagePayload)",
]);
const fee = new Interface([
  "function getMessageInfo(bytes32) view returns((address payer,uint32 destinationChain,uint256 gasLimit,uint256 relayFee,uint256 tip,uint256 tipExpiry,uint256 deliveryDeadline,bool relaySettled,bool feesInNative))",
  "event DeliveryClaimed(bytes32 indexed messageId,address indexed relayer,address indexed submitter,uint256 relayFee,uint256 tip)",
]);
export const PUBLICATION_TOPIC = id(
  "MessagePublished(bytes32,bytes32,uint64,bool,bytes)",
);

interface Log {
  address: string;
  topics: string[];
  data: string;
  blockNumber: string;
  transactionHash: string;
  blockHash: string;
  removed?: boolean;
}
interface Receipt {
  status: string;
  transactionHash: string;
  blockNumber: string;
  blockHash: string;
  logs: Log[];
}
export interface Publication {
  messageId: string;
  block: number;
  transactionHash: string;
  payload: string;
  emitter: string;
  outbox: string;
}

export function requireEvidence(
  condition: unknown,
  message: string,
): asserts condition {
  if (!condition) throw new FatalError(message);
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const hash = (bytes: string) =>
  createHash("sha256").update(bytes).digest("hex");

async function call(
  rpc: Rpc,
  address: string,
  abi: Interface,
  method: string,
  args: unknown[],
) {
  const result = await rpc.call<string>("eth_call", [
    { to: address, data: abi.encodeFunctionData(method, args) },
    "latest",
  ]);
  return abi.decodeFunctionResult(method, result);
}

async function logs(
  rpc: Rpc,
  address: string,
  topic: string,
  messageId: string,
  from: number,
  to = "latest",
): Promise<Log[]> {
  const found = await rpc.call<Log[]>("eth_getLogs", [
    {
      address,
      topics: [topic, messageId],
      fromBlock: toQuantity(from),
      toBlock: to,
    },
  ]);
  for (const log of found) {
    requireEvidence(
      same(log.address, address) &&
        log.topics[0] === topic &&
        same(log.topics[1] ?? "", messageId),
      "RPC returned an event for another contract, event type or message ID",
    );
  }
  return found;
}

async function checkedReceipt(
  rpc: Rpc,
  log: Log,
  address: string,
): Promise<Receipt> {
  requireEvidence(
    !log.removed && same(log.address, address),
    "event has wrong emitter or was removed",
  );
  const receipt = await rpc.call<Receipt | null>("eth_getTransactionReceipt", [
    log.transactionHash,
  ]);
  requireEvidence(
    receipt && receipt.status === "0x1",
    "event has no successful transaction receipt",
  );
  requireEvidence(
    same(receipt.transactionHash, log.transactionHash) &&
      same(receipt.blockHash, log.blockHash) &&
      receipt.blockNumber === log.blockNumber,
    "event receipt provenance disagrees",
  );
  requireEvidence(
    receipt.logs.some(
      (candidate) =>
        same(candidate.address, address) &&
        candidate.data === log.data &&
        JSON.stringify(candidate.topics) === JSON.stringify(log.topics),
    ),
    "receipt does not contain the asserted event",
  );
  return receipt;
}

export async function publish(
  stack: Stack,
  name: string,
): Promise<Publication> {
  const path = resolve(stack.state, `published-${name}.json`);
  await stack.script(
    `publish-${name}`,
    "scripts/publish-fee.mts",
    [name],
    { ...stack.env, E2E_PUBLISHED_PATH: path },
    180_000,
  );
  const published = await readJson<{ messageId: string; block: number }>(path);
  requireEvidence(
    /^0x[0-9a-fA-F]{64}$/.test(published.messageId) &&
      Number.isSafeInteger(published.block),
    "publisher did not return a message ID and block",
  );
  const matches = await logs(
    stack.source,
    stack.deployment.source.outbox,
    PUBLICATION_TOPIC,
    published.messageId,
    published.block,
    toQuantity(published.block),
  );
  requireEvidence(
    matches.length === 1,
    `expected exactly one publication for ${published.messageId}, got ${matches.length}`,
  );
  await checkedReceipt(
    stack.source,
    matches[0],
    stack.deployment.source.outbox,
  );
  const event = outbox.parseLog(matches[0]);
  requireEvidence(
    event?.name === "MessagePublished" && event.args.canAck === true,
    "publication must request acknowledgment",
  );
  const result: Publication = {
    ...published,
    transactionHash: matches[0].transactionHash,
    payload: event.args.payload as string,
    emitter: getAddress(`0x${String(event.args.emitterAddress).slice(2, 42)}`),
    outbox: stack.deployment.source.outbox,
  };
  requireEvidence(
    same(result.emitter, new Wallet(SOURCE_KEY).address),
    "publication has wrong source emitter",
  );
  await stack.event("published", result);
  return result;
}

export async function assertSettled(
  stack: Stack,
  publication: Publication,
): Promise<unknown> {
  const messageId = publication.messageId;
  const execution = await stack.wait(
    `execution of ${messageId}`,
    async () => {
      const [processed] = await call(
        stack.destination,
        stack.deployment.dest.inbox,
        inbox,
        "processedAt",
        [messageId],
      );
      if (processed === 0n) return undefined;
      const block = Number(processed);
      const found = await logs(
        stack.destination,
        stack.deployment.dest.inbox,
        inbox.getEvent("MessageExecuted")!.topicHash,
        messageId,
        block,
        toQuantity(block),
      );
      requireEvidence(
        found.length === 1,
        "processedAt must identify exactly one matching MessageExecuted",
      );
      await checkedReceipt(
        stack.destination,
        found[0],
        stack.deployment.dest.inbox,
      );
      const event = inbox.parseLog(found[0])!;
      requireEvidence(
        same(event.args.messageId, messageId) &&
          same(event.args.emitterAddress, publication.emitter) &&
          same(event.args.destination, stack.deployment.dest.dapp) &&
          event.args.messagePayload === publication.payload &&
          same(event.args.relayer, new Wallet(ANVIL_KEY).address),
        "execution event does not match the exact published message, payload and route",
      );
      return { block, transactionHash: found[0].transactionHash, messageId };
    },
    420_000,
  );
  const acknowledgment = await stack.wait(
    `acknowledgment of ${messageId}`,
    async () => {
      const [acknowledged] = await call(
        stack.source,
        publication.outbox,
        outbox,
        "isAcknowledged",
        [messageId],
      );
      if (!acknowledged) return undefined;
      const found = await logs(
        stack.source,
        publication.outbox,
        outbox.getEvent("MessageAcknowledged")!.topicHash,
        messageId,
        publication.block,
      );
      requireEvidence(
        found.length === 1,
        "acknowledged state must have exactly one matching MessageAcknowledged",
      );
      await checkedReceipt(stack.source, found[0], publication.outbox);
      return {
        transactionHash: found[0].transactionHash,
        messageId,
        outbox: publication.outbox,
      };
    },
    420_000,
  );
  const settlement = await stack.wait(
    `automatic relay fee claim for ${messageId}`,
    async () => {
      const [info] = await call(
        stack.source,
        stack.deployment.source.relayerContract,
        fee,
        "getMessageInfo",
        [messageId],
      );
      if (!info.relaySettled) return undefined;
      const found = await logs(
        stack.source,
        stack.deployment.source.relayerContract,
        fee.getEvent("DeliveryClaimed")!.topicHash,
        messageId,
        publication.block,
      );
      requireEvidence(
        found.length === 1,
        "settled ledger must have exactly one matching DeliveryClaimed",
      );
      await checkedReceipt(
        stack.source,
        found[0],
        stack.deployment.source.relayerContract,
      );
      const event = fee.parseLog(found[0])!;
      requireEvidence(
        event.args.relayFee === info.relayFee &&
          event.args.tip === info.tip &&
          info.relayFee > 0n,
        "claimed relay fee and tip must equal the funded message ledger",
      );
      requireEvidence(
        same(event.args.relayer, new Wallet(ANVIL_KEY).address) &&
          same(event.args.submitter, new Wallet(SOURCE_KEY).address),
        "wrong fee beneficiary or automatic claim submitter",
      );
      return {
        transactionHash: found[0].transactionHash,
        messageId,
        relayFee: info.relayFee.toString(),
        tip: info.tip.toString(),
        relayer: event.args.relayer as string,
      };
    },
    420_000,
  );
  return { publication, execution, acknowledgment, settlement };
}

export function validateDurableState(
  checkpoint: unknown,
  journal: unknown,
  publication: Publication,
  lookback: number,
): number {
  const entry = (
    checkpoint as Record<string, { block?: number; durable_work?: boolean }>
  )["outboxes:2"];
  requireEvidence(
    entry?.durable_work === true && Number.isSafeInteger(entry.block),
    "source checkpoint lacks durable all-Outbox coverage",
  );
  requireEvidence(
    entry.block! - lookback > publication.block,
    "publication is still inside the source checkpoint lookback window",
  );
  const record = journal as {
    message_id?: string;
    block_height?: number;
    chain_key?: number;
    outbox?: string;
    tx_hash?: string;
  };
  requireEvidence(
    record.message_id &&
      same(record.message_id, publication.messageId) &&
      record.chain_key === 2 &&
      record.block_height === publication.block &&
      record.outbox &&
      same(record.outbox, publication.outbox) &&
      record.tx_hash &&
      same(record.tx_hash, publication.transactionHash),
    "durable journal does not match the exact publication",
  );
  return entry.block!;
}

export function assertNoPublicationReplay(
  scans: Scan[],
  publication: Publication,
): void {
  const publicationScans = scans.filter(({ phase, filter }) => {
    const topics = filter.topics as unknown[] | undefined;
    // Ack routing may look up this exact publication by topic1. Only broad source indexing can
    // re-admit it and mask a lost journal, so distinguish those legitimate historical lookups.
    return (
      phase === "restarted" &&
      topics?.[0] === PUBLICATION_TOPIC &&
      topics[1] == null
    );
  });
  requireEvidence(
    publicationScans.length > 0,
    "no restarted source scan was observed",
  );
  for (const { filter } of publicationScans) {
    requireEvidence(
      typeof filter.fromBlock === "string" &&
        /^0x[0-9a-f]+$/i.test(filter.fromBlock) &&
        BigInt(filter.fromBlock) > BigInt(publication.block),
      "restarted source indexing replayed the old publication",
    );
  }
}

export async function happyPath(stack: Stack): Promise<unknown> {
  return assertSettled(stack, await publish(stack, "happy-path"));
}

export async function durableRestart(stack: Stack): Promise<unknown> {
  stack.destinationProxy.gate(true);
  const blockedBefore = stack.destinationProxy.stats.rejected;
  const publication = await publish(stack, "durable-restart");
  const journalPath = `${stack.checkpoint}.messages/2/${publication.messageId}.json`;
  await stack.wait(
    "journal and source cursor beyond restart lookback",
    async () => {
      const checkpoint = await readJson<
        Record<string, { block?: number; durable_work?: boolean }>
      >(stack.checkpoint);
      if (
        (checkpoint["outboxes:2"]?.block ?? 0) - stack.lookback <=
        publication.block
      )
        return undefined;
      return validateDurableState(
        checkpoint,
        await readJson(journalPath),
        publication,
        stack.lookback,
      );
    },
  );
  await stack.wait("destination fault observed by actual relayer", async () =>
    stack.destinationProxy.stats.rejected > blockedBefore ? true : undefined,
  );
  const [before] = await call(
    stack.destination,
    stack.deployment.dest.inbox,
    inbox,
    "processedAt",
    [publication.messageId],
  );
  requireEvidence(before === 0n, "message executed before the crash boundary");
  const killedPid = stack.relayer!.child.pid;
  const exit = await stack.relayer!.stop("SIGKILL");
  requireEvidence(
    exit.signal === "SIGKILL",
    "durable restart did not terminate the real relayer with SIGKILL",
  );
  const checkpointBytes = await readFile(stack.checkpoint, "utf8");
  const journalBytes = await readFile(journalPath, "utf8");
  const configBytes = await readFile(stack.configPath, "utf8");
  const cursor = validateDurableState(
    JSON.parse(checkpointBytes),
    JSON.parse(journalBytes),
    publication,
    stack.lookback,
  );
  // Preserve crash evidence separately. Never edit, rewind or remove the live checkpoint/journal.
  await writeFile(
    resolve(stack.output, "crash-checkpoint.json"),
    checkpointBytes,
  );
  await writeFile(resolve(stack.output, "crash-journal.json"), journalBytes);
  const finalizedAfterCrash = await stack.wait(
    "source finality advancing while relayer is dead",
    async () => {
      const finalized = await stack.source.call<{ number: string } | null>(
        "eth_getBlockByNumber",
        ["finalized", false],
      );
      return finalized &&
        Number(BigInt(finalized.number)) > cursor + stack.lookback
        ? Number(BigInt(finalized.number))
        : undefined;
    },
  );
  requireEvidence(
    (await readFile(stack.checkpoint, "utf8")) === checkpointBytes &&
      (await readFile(journalPath, "utf8")) === journalBytes &&
      (await readFile(stack.configPath, "utf8")) === configBytes,
    "persistent restart inputs changed while relayer was dead",
  );
  stack.sourceProxy.phase = "restarted";
  await stack.startRelayer();
  requireEvidence(
    stack.relayer!.child.pid !== killedPid,
    "relayer PID did not change across restart",
  );
  const crash = {
    killedPid,
    restartedPid: stack.relayer!.child.pid,
    exit,
    sourceCursor: cursor,
    publicationBlock: publication.block,
    lookback: stack.lookback,
    finalizedAfterCrash,
    checkpointSha256: hash(checkpointBytes),
    journalSha256: hash(journalBytes),
    configSha256: hash(configBytes),
    rejectedDestinationCalls:
      stack.destinationProxy.stats.rejected - blockedBefore,
  };
  await stack.event("durable-restart", crash);
  stack.destinationProxy.gate(false);
  const settled = await assertSettled(stack, publication);
  assertNoPublicationReplay(stack.sourceProxy.scans, publication);
  await writeFile(
    resolve(stack.output, "restart-source-scans.json"),
    json(stack.sourceProxy.scans.filter((scan) => scan.phase === "restarted")),
  );
  return { crash, settled };
}
