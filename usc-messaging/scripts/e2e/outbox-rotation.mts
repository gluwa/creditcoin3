import { readdir, readFile, readlink } from "node:fs/promises";
import { setTimeout as delay } from "node:timers/promises";
import { ApiPromise, WsProvider } from "@polkadot/api";
import {
  type ContractTransactionResponse,
  Contract,
  Interface,
  JsonRpcProvider,
  type Log,
  NonceManager,
  Wallet,
  ZeroAddress,
  getAddress,
  getBytes,
  keccak256,
  parseEther,
} from "ethers";
import { memoEnvelope } from "../evm-envelope.mjs";
import { computeMessageId } from "./message-id.mjs";
import { FatalError } from "./runtime.mjs";
import {
  PUBLICATION_TOPIC,
  requireEvidence,
  type Publication,
} from "./scenarios.mjs";
import { ANVIL_KEY, SOURCE_KEY, type OutboxSet, type Stack } from "./stack.mjs";

// Deterministic test keys used only to publish. They are funded during deployment, and keep the
// publishers' nonces apart from the relayer's settlement signer, which shares the deployer key.
const FOLLOWER_KEY = `0x${"f0".repeat(32)}`;
const PINNED_KEY = `0x${"a1".repeat(32)}`;
export const PUBLISHER_ADDRESSES = [FOLLOWER_KEY, PINNED_KEY].map(
  (key) => new Wallet(key).address,
);

const SOURCE_CHAIN_ID = 42n;
const QUOTE_GAS_LIMIT = 500_000;
const PACE = Number(process.env.E2E_FAST_FORWARD_BLOCKS_PER_SECOND ?? 10);

const outboxAbi = new Interface([
  "event MessagePublished(bytes32 indexed messageId,bytes32 indexed emitterAddress,uint64 sequence,bool canAck,bytes payload)",
  "function approveForwarder(address,bool)",
]);
const discoveryAbi = [
  "function defaultOutbox(uint32) view returns (address)",
  "function isActiveOutbox(uint32,address) view returns (bool)",
  "function timelock() view returns (uint64)",
  "function setDefaultOutbox(uint32,address)",
  "function removeOutbox(uint32,address,address)",
];
const inboxAbi = new Interface([
  "function processedAt(bytes32) view returns (uint256)",
  "event MessageExecuted(bytes32 indexed messageId,address indexed emitterAddress,address indexed destination,address dispatcher,address relayer,bytes messagePayload)",
]);

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

interface Target extends OutboxSet {
  name: "A" | "B";
  quoterUrl: string;
}

interface Sent {
  publisher: string;
  target: Target;
  memo: string;
  hash: string;
  /** Head when the publisher chose its Outbox. */
  resolvedAt: number;
}

/** Publishes fee-paying messages from the controller, so a block can be chosen for each one. */
class Publisher {
  readonly signer: NonceManager;
  readonly address: string;
  private readonly ready = new Set<string>();
  readonly confirmed: (Sent & Publication)[] = [];

  constructor(
    private readonly stack: Stack,
    readonly name: string,
    key: string,
    private readonly provider: JsonRpcProvider,
  ) {
    const wallet = new Wallet(key, provider);
    this.address = wallet.address;
    this.signer = new NonceManager(wallet);
  }

  /** One-time approvals for an Outbox; needs blocks, so call it while the sealer is ticking. */
  async prepare(target: Target): Promise<void> {
    if (this.ready.has(target.name)) return;
    const attest = new Contract(
      this.stack.deployment.source.attest,
      ["function mint(address,uint256)", "function approve(address,uint256)"],
      this.signer,
    );
    const funds = parseEther("100000");
    await (await attest.mint(this.address, funds)).wait();
    await (await attest.approve(target.relayerContract, funds)).wait();
    const outbox = new Contract(target.outbox, outboxAbi, this.signer);
    await (await outbox.approveForwarder(target.relayerContract, true)).wait();
    this.ready.add(target.name);
  }

  /** Submits one publication without waiting for a block. */
  async send(target: Target, memo: string): Promise<Sent> {
    requireEvidence(this.ready.has(target.name), "publisher is not prepared");
    const resolvedAt = await this.provider.getBlockNumber();
    const payload = getBytes(
      memoEnvelope(this.stack.deployment.dest.dapp, `rotation ${memo}`),
    );
    const response = await fetch(`${target.quoterUrl}/quote`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        destinationChain: this.stack.deployment.source.chainKey,
        targetContract: this.address,
        payloadHash: keccak256(payload),
        gasLimit: QUOTE_GAS_LIMIT,
        requiresAck: true,
      }),
      signal: AbortSignal.timeout(10_000),
    });
    const body = (await response.json()) as { signedQuote?: string };
    if (!response.ok || typeof body.signedQuote !== "string")
      throw new Error(
        `quoter HTTP ${response.status}: ${JSON.stringify(body)}`,
      );
    const relayer = new Contract(
      target.relayerContract,
      [
        "function publishAndCollectRelayerFee(bytes,bytes,uint256,uint256) returns (bytes32)",
      ],
      this.signer,
    );
    // No tip: its expiry is wall-clock sensitive and is not what this drill is about.
    const tx = await relayer.publishAndCollectRelayerFee(
      payload,
      body.signedQuote,
      0,
      0,
    );
    return { publisher: this.name, target, memo, hash: tx.hash, resolvedAt };
  }

  /** Reads back a mined publication from its receipt. */
  async confirm(sent: Sent): Promise<Sent & Publication> {
    // Polled directly: on a held chain no new block arrives to wake ethers' own waiter.
    const receipt = await this.stack.wait(
      `receipt of publication ${sent.memo}`,
      async () =>
        (await this.provider.getTransactionReceipt(sent.hash)) ?? undefined,
      60_000,
    );
    requireEvidence(
      receipt?.status === 1,
      `publication ${sent.memo} (${sent.hash}) was not mined successfully`,
    );
    const events = receipt.logs
      .filter(
        (log) =>
          same(log.address, sent.target.outbox) &&
          log.topics[0] === PUBLICATION_TOPIC,
      )
      .map((log) => outboxAbi.parseLog(log)!);
    requireEvidence(
      events.length === 1,
      `publication ${sent.memo} emitted ${events.length} MessagePublished events`,
    );
    const publication = {
      ...sent,
      messageId: events[0].args.messageId as string,
      block: receipt.blockNumber,
      transactionHash: receipt.hash,
      payload: events[0].args.payload as string,
      emitter: this.address,
      outbox: sent.target.outbox,
      relayerContract: sent.target.relayerContract,
    };
    this.confirmed.push(publication);
    await this.stack.event("rotation-published", {
      publisher: this.name,
      memo: sent.memo,
      outbox: sent.target.name,
      block: publication.block,
      messageId: publication.messageId,
    });
    return publication;
  }

  async publish(target: Target, memo: string): Promise<Sent & Publication> {
    return this.confirm(await this.send(target, memo));
  }
}

/**
 * SIGKILLs one attestor of the fleet and starts it again with the same command line, working
 * directory and environment. The launcher tolerates a child exiting, so the other three keep
 * running and the restarted one rejoins through its original boot nodes.
 */
async function restartAttestor(stack: Stack, index: number): Promise<unknown> {
  const wanted = [
    `--name=zombie-${index}`,
    `--config=${stack.env.E2E_ATTESTOR_CONFIG}`,
  ];
  const matches: { pid: number; argv: string[] }[] = [];
  for (const entry of await readdir("/proc")) {
    if (!/^\d+$/.test(entry)) continue;
    const argv = await readFile(`/proc/${entry}/cmdline`, "utf8")
      .then((raw) => raw.split("\0").filter(Boolean))
      .catch((): string[] => []);
    if (wanted.every((arg) => argv.includes(arg)))
      matches.push({ pid: Number(entry), argv });
  }
  requireEvidence(
    matches.length === 1,
    `expected one running attestor ${index}, found ${matches.length}`,
  );
  const found = matches[0];
  const cwd = await readlink(`/proc/${found.pid}/cwd`);
  const env = Object.fromEntries(
    (await readFile(`/proc/${found.pid}/environ`, "utf8"))
      .split("\0")
      .filter((pair) => pair.includes("="))
      .map((pair) => [
        pair.slice(0, pair.indexOf("=")),
        pair.slice(pair.indexOf("=") + 1),
      ]),
  );
  process.kill(found.pid, "SIGKILL");
  await stack.wait(
    `attestor ${index} exit`,
    async () =>
      (await readFile(`/proc/${found.pid}/cmdline`).catch(() => undefined))
        ? undefined
        : true,
    30_000,
  );
  const child = stack.start(
    `attestor-${index}-restarted-at-${Date.now()}`,
    found.argv[0],
    found.argv.slice(1),
    env,
    cwd,
  );
  const restart = {
    index,
    killedPid: found.pid,
    restartedPid: child.child.pid,
  };
  await stack.event("attestor-restarted", restart);
  return restart;
}

/**
 * Block numbers that start a BABE epoch. Their session change uses most of the block weight, so
 * a publication submitted for one of them is deferred to the next block. A manually sealed chain
 * advances one slot per block, which makes every later epoch start predictable.
 */
async function epochStarts(stack: Stack): Promise<(block: number) => boolean> {
  const api = await ApiPromise.create({
    provider: new WsProvider(stack.sourceWs),
    noInitWarn: true,
  });
  try {
    const duration = Number(api.consts.babe.epochDuration.toString());
    const [, current] = (await api.query.babe.epochStart()).toJSON() as [
      number,
      number,
    ];
    requireEvidence(
      Number.isSafeInteger(duration) && duration > 3 && current >= 0,
      "could not read the BABE epoch schedule",
    );
    return (block) => (block - current) % duration === 0;
  } finally {
    await api.disconnect();
  }
}

const feeAbi = new Interface([
  "function getMessageInfo(bytes32) view returns((address payer,uint32 destinationChain,uint256 gasLimit,uint256 relayFee,uint256 tip,uint256 tipExpiry,uint256 deliveryDeadline,bool relaySettled,bool feesInNative))",
  "event DeliveryClaimed(bytes32 indexed messageId,address indexed relayer,address indexed submitter,uint256 relayFee,uint256 tip)",
]);
const ackAbi = new Interface([
  "event MessageAcknowledged(bytes32 indexed messageId)",
  "function isAcknowledged(bytes32) view returns(bool)",
]);
const ACK_TOPIC = ackAbi.getEvent("MessageAcknowledged")!.topicHash;
const CLAIM_TOPIC = feeAbi.getEvent("DeliveryClaimed")!.topicHash;

/**
 * Collects raw source-chain logs while the drill runs. The node keeps only recent history, so
 * the evidence has to be read as blocks are sealed; a scan at the end would miss the early
 * publications. Nothing here comes from the relayer, the attestors or the publishers.
 */
class SourceLogs {
  readonly logs: Log[] = [];
  private next: number;
  private stopped = false;
  private failure?: unknown;
  private readonly loop: Promise<void>;

  constructor(
    private readonly provider: JsonRpcProvider,
    private readonly addresses: string[],
    from: number,
  ) {
    this.next = from;
    this.loop = this.run();
  }

  private async scan(): Promise<void> {
    const head = await this.provider.getBlockNumber();
    while (this.next <= head) {
      const to = Math.min(this.next + 99, head);
      this.logs.push(
        ...(await this.provider.getLogs({
          address: this.addresses,
          topics: [[PUBLICATION_TOPIC, ACK_TOPIC, CLAIM_TOPIC]],
          fromBlock: this.next,
          toBlock: to,
        })),
      );
      this.next = to + 1;
    }
  }

  private async run(): Promise<void> {
    while (!this.stopped) {
      try {
        await this.scan();
      } catch (error) {
        this.failure = error;
        return;
      }
      await delay(300);
    }
  }

  assertAlive(): void {
    if (this.failure)
      throw new FatalError(`source log scan failed: ${String(this.failure)}`);
  }

  /** Stops the loop and reads up to the current head. */
  async finish(): Promise<Log[]> {
    this.stopped = true;
    await this.loop;
    this.assertAlive();
    await this.scan();
    return this.logs;
  }
}

interface LedgerEntry {
  messageId: string;
  block: number;
  emitter: string;
  payload: string;
  target: Target;
  /** False for a publication on an Outbox that Discovery no longer authorized at that block. */
  authorized: boolean;
}

/** Every publication on both Outboxes, with each message ID recomputed from the logged fields. */
function buildLedger(
  logs: Log[],
  targets: Target[],
  removal: { outbox: string; effectiveBlock: number },
): LedgerEntry[] {
  return logs
    .filter((log) => log.topics[0] === PUBLICATION_TOPIC)
    .map((log) => {
      const target = targets.find((t) => same(t.outbox, log.address));
      const event = outboxAbi.parseLog(log);
      requireEvidence(
        target && event && !log.removed,
        "publication log from an unknown Outbox",
      );
      // The event carries the emitter as bytes32 with the address in the leading 20 bytes.
      const emitter = getAddress(
        `0x${String(event.args.emitterAddress).slice(2, 42)}`,
      );
      const messageId = event.args.messageId as string;
      requireEvidence(
        computeMessageId(
          target.outbox,
          emitter,
          event.args.sequence as bigint,
          event.args.payload as string,
          SOURCE_CHAIN_ID,
        ) === messageId,
        `Outbox logged a message ID that does not match its own fields: ${messageId}`,
      );
      return {
        messageId,
        block: log.blockNumber,
        emitter,
        payload: event.args.payload as string,
        target,
        authorized: !(
          same(target.outbox, removal.outbox) &&
          log.blockNumber >= removal.effectiveBlock
        ),
      };
    });
}

/**
 * The defaultOutbox rotation drill. Two Outboxes serve one chain key. The default moves from A
 * to B through `setDefaultOutbox`, then A is retired through `removeOutbox`, each across a real
 * 1200-block timelock. Publishers keep sending throughout, at chosen blocks around both
 * boundaries, while the relayer and attestors are killed and restarted. The ledger check then
 * proves from raw logs that every authorized message was delivered exactly once, acknowledged on
 * its own Outbox and had its relay fee claimed, and that nothing else was delivered.
 */
export async function outboxRotation(stack: Stack): Promise<unknown> {
  const sealer = stack.sealer;
  const second = stack.deployment.source.second;
  if (!sealer || !second)
    throw new FatalError(
      "outbox-rotation needs a manual stack with two Outboxes",
    );
  const chainKey = stack.deployment.source.chainKey;
  const A: Target = {
    name: "A",
    outbox: stack.deployment.source.outbox,
    ackValidator: stack.deployment.source.ackValidator,
    relayerContract: stack.deployment.source.relayerContract,
    quoterUrl: `http://127.0.0.1:${stack.quoterPort}`,
  };
  const B: Target = {
    name: "B",
    ...second,
    quoterUrl: `http://127.0.0.1:${stack.secondQuoterPort}`,
  };
  const provider = new JsonRpcProvider(
    stack.source.url,
    Number(SOURCE_CHAIN_ID),
    {
      staticNetwork: true,
      cacheTimeout: -1,
    },
  );
  provider.pollingInterval = 250;
  const destination = new JsonRpcProvider(stack.destination.url, undefined, {
    staticNetwork: true,
    cacheTimeout: -1,
  });
  try {
    const owner = new Wallet(SOURCE_KEY, provider);
    const discovery = new Contract(
      stack.deployment.source.outboxDiscovery,
      discoveryAbi,
      owner,
    );
    const currentDefault = async (): Promise<Target> => {
      const address = (await discovery.defaultOutbox(chainKey)) as string;
      const target = [A, B].find((t) => same(t.outbox, address));
      requireEvidence(target, `Discovery default ${address} is not A or B`);
      return target;
    };
    const timelock = Number(await discovery.timelock());
    requireEvidence(
      timelock >= 1200,
      "Discovery timelock is below the minimum",
    );
    requireEvidence(
      (await currentDefault()).name === "A",
      "the drill must start with A as the default",
    );

    // One publisher asks Discovery for the default before every message; the other never
    // leaves A.
    const follower = new Publisher(stack, "follower", FOLLOWER_KEY, provider);
    const pinned = new Publisher(stack, "pinned-A", PINNED_KEY, provider);
    const sourceLogs = new SourceLogs(
      provider,
      [A.outbox, B.outbox, A.relayerContract, B.relayerContract],
      await provider.getBlockNumber(),
    );
    await follower.prepare(A);
    await follower.prepare(B);
    await pinned.prepare(A);
    const follow = async (memo: string) =>
      follower.publish(await currentDefault(), memo);
    /** Places one publication in the next block of a held chain and proves where it landed. */
    const place = async (
      publisher: Publisher,
      target: Target,
      memo: string,
      block: number,
    ) => {
      const sent = await publisher.send(target, memo);
      const sealed = await sealer.sealOne();
      const publication = await publisher.confirm(sent);
      requireEvidence(
        sealed === block && publication.block === block,
        `${memo} landed in block ${publication.block}, not ${block}`,
      );
      return publication;
    };
    const restartRelayer = async (whileDown: () => Promise<void>) => {
      const exit = await stack.relayer!.stop("SIGKILL");
      requireEvidence(exit.signal === "SIGKILL", "relayer was not killed");
      await whileDown();
      await stack.startRelayer();
    };
    const restarts: unknown[] = [];
    const startsEpoch = await epochStarts(stack);
    /**
     * Mines a timelocked Discovery change in a block chosen so that the change's effective
     * block and its two neighbours can each carry a publication. Returns the effective block.
     */
    const schedule = async (
      send: () => Promise<ContractTransactionResponse>,
    ): Promise<number> => {
      let next = (await sealer.pause()) + 1;
      const usable = (at: number) =>
        [at, at + timelock - 1, at + timelock, at + timelock + 1].every(
          (block) => !startsEpoch(block),
        );
      while (!usable(next)) next = (await sealer.sealOne()) + 1;
      const tx = await send();
      const sealed = await sealer.sealOne();
      const receipt = await stack.wait(
        "receipt of the Discovery change",
        async () =>
          (await provider.getTransactionReceipt(tx.hash)) ?? undefined,
        60_000,
      );
      sealer.release();
      requireEvidence(
        receipt.status === 1 &&
          receipt.blockNumber === sealed &&
          sealed === next,
        "the Discovery change was not mined in its chosen block",
      );
      return sealed + timelock;
    };

    // Path 1: move the default from A to B. The relayer has nothing to settle yet, so the
    // owner key it shares for settlement is free.
    const defaultChange = await schedule(() =>
      discovery.setDefaultOutbox(chainKey, B.outbox),
    );
    await stack.event("default-change-scheduled", {
      effectiveBlock: defaultChange,
    });
    await follow("after-scheduling-1");
    await pinned.publish(A, "after-scheduling-1");
    await follow("after-scheduling-2");

    // Restart before the boundary, with a publication while the relayer is down.
    await restartRelayer(async () => {
      restarts.push(await restartAttestor(stack, 3));
      await follow("relayer-down-before-default-change");
    });

    await sealer.holdAt(defaultChange - 2, PACE);
    requireEvidence(
      (await currentDefault()).name === "A",
      "the default moved before its effective block",
    );
    await place(
      follower,
      await currentDefault(),
      "last-before-default-change",
      defaultChange - 1,
    );
    // Resolved while A is still the default, mined in the block where B takes over. A stays
    // authorized, so this message must still be delivered.
    const straddling = await place(
      follower,
      await currentDefault(),
      "resolved-before-mined-at-default-change",
      defaultChange,
    );
    requireEvidence(
      straddling.target.name === "A" && (await currentDefault()).name === "B",
      "the default did not move from A to B exactly at its effective block",
    );
    await place(
      follower,
      await currentDefault(),
      "first-after-default-change",
      defaultChange + 1,
    );
    sealer.release();
    await follow("after-default-change");
    await pinned.publish(A, "after-default-change");

    // Path 2: retire A. Scheduled while the relayer is down, so the owner key is free again.
    let removal = 0;
    await restartRelayer(async () => {
      removal = await schedule(() =>
        discovery.removeOutbox(chainKey, A.outbox, ZeroAddress),
      );
      await stack.event("removal-scheduled", { effectiveBlock: removal });
      restarts.push(await restartAttestor(stack, 2));
      await follow("relayer-down-before-removal");
      await pinned.publish(A, "relayer-down-before-removal");
    });
    await follow("after-removal-scheduled");

    await sealer.holdAt(removal - 2, PACE);
    requireEvidence(
      await discovery.isActiveOutbox(chainKey, A.outbox),
      "A stopped being active before its removal block",
    );
    await place(pinned, A, "last-authorized-on-A", removal - 1);
    await place(pinned, A, "first-unauthorized-on-A", removal);
    requireEvidence(
      !(await discovery.isActiveOutbox(chainKey, A.outbox)) &&
        (await discovery.isActiveOutbox(chainKey, B.outbox)),
      "A was not retired exactly at its removal block",
    );
    await place(
      follower,
      await currentDefault(),
      "first-after-removal",
      removal + 1,
    );
    sealer.release();
    await pinned.publish(A, "unauthorized-after-removal");
    await follow("after-removal");

    // Restart after the boundary, then one last message published behind everything else: once
    // it has settled, the relayer and attestors have seen every earlier block.
    await restartRelayer(async () => {
      restarts.push(await restartAttestor(stack, 1));
      await follow("relayer-down-after-removal");
    });
    await follow("canary");

    const inbox = new Contract(
      stack.deployment.dest.inbox,
      inboxAbi,
      destination,
    );
    const sent = [...follower.confirmed, ...pinned.confirmed];
    const state = async (entry: { messageId: string; target: Target }) => {
      const [executedAt, acknowledged, info] = await Promise.all([
        inbox.processedAt(entry.messageId) as Promise<bigint>,
        new Contract(entry.target.outbox, ackAbi, provider).isAcknowledged(
          entry.messageId,
        ) as Promise<boolean>,
        new Contract(
          entry.target.relayerContract,
          feeAbi,
          provider,
        ).getMessageInfo(entry.messageId),
      ]);
      return { executedAt, acknowledged, info };
    };
    // Let everything drain, bounded. The canary was published behind every other message, so
    // once it has settled the relayer and attestors have seen every earlier block. A message
    // that never settles is reported below with the rest instead of ending the drill here.
    await stack
      .wait(
        "every authorized message delivered, acknowledged and claimed",
        async () => {
          sourceLogs.assertAlive();
          for (const publication of sent) {
            if (
              same(publication.outbox, A.outbox) &&
              publication.block >= removal
            )
              continue;
            const { executedAt, acknowledged, info } = await state(publication);
            if (executedAt === 0n || !acknowledged || !info.relaySettled)
              return undefined;
          }
          return true;
        },
        900_000,
      )
      .catch((error: unknown) => {
        if (error instanceof FatalError) throw error;
      });
    // A few more blocks and some wall time, so an unauthorized delivery would have happened.
    await sealer.fastForward((await sealer.height()) + 30, PACE);
    await delay(20_000, undefined, { signal: stack.signal });

    const logs = await sourceLogs.finish();
    const ledger = buildLedger(logs, [A, B], {
      outbox: A.outbox,
      effectiveBlock: removal,
    });
    const expected = ledger.filter((entry) => entry.authorized);
    const refused = ledger.filter((entry) => !entry.authorized);
    requireEvidence(
      ledger.length === sent.length &&
        sent.every((p) => ledger.some((e) => e.messageId === p.messageId)),
      `the on-chain ledger has ${ledger.length} publications but the publishers sent ${sent.length}`,
    );
    requireEvidence(
      refused.length === 2 && expected.some((e) => e.block === removal - 1),
      "the drill did not produce the boundary publications it is meant to test",
    );

    const executed = await destination.getLogs({
      address: stack.deployment.dest.inbox,
      topics: [inboxAbi.getEvent("MessageExecuted")!.topicHash],
      fromBlock: 0,
      toBlock: "latest",
    });
    const count = (found: Log[], topic: string, address: string, id: string) =>
      found.filter(
        (log) =>
          log.topics[0] === topic &&
          same(log.address, address) &&
          log.topics[1] === id,
      );
    const relayer = new Wallet(ANVIL_KEY).address;
    const failures: {
      messageId: string;
      outbox: string;
      block: number;
      why: string;
    }[] = [];
    const fail = (entry: LedgerEntry, why: string) =>
      failures.push({
        messageId: entry.messageId,
        outbox: entry.target.name,
        block: entry.block,
        why,
      });
    const executedTopic = inboxAbi.getEvent("MessageExecuted")!.topicHash;
    for (const entry of expected) {
      const { executedAt, acknowledged, info } = await state(entry);
      const deliveries = count(
        executed,
        executedTopic,
        stack.deployment.dest.inbox,
        entry.messageId,
      );
      if (executedAt === 0n || deliveries.length !== 1) {
        fail(entry, `delivered ${deliveries.length} times`);
      } else {
        const event = inboxAbi.parseLog(deliveries[0])!;
        if (
          event.args.messagePayload !== entry.payload ||
          !same(event.args.emitterAddress, entry.emitter) ||
          !same(event.args.relayer, relayer)
        )
          fail(entry, "delivered with a different payload, emitter or relayer");
      }
      const acks = count(logs, ACK_TOPIC, entry.target.outbox, entry.messageId);
      if (!acknowledged || acks.length !== 1)
        fail(entry, `acknowledged ${acks.length} times on its own Outbox`);
      const claims = count(
        logs,
        CLAIM_TOPIC,
        entry.target.relayerContract,
        entry.messageId,
      );
      if (!info.relaySettled || claims.length !== 1) {
        fail(
          entry,
          `relay fee claimed ${claims.length} times in its own ledger`,
        );
      } else {
        const claim = feeAbi.parseLog(claims[0])!;
        if (
          claim.args.relayFee !== info.relayFee ||
          info.relayFee === 0n ||
          !same(claim.args.relayer, relayer)
        )
          fail(entry, "claimed fee or beneficiary does not match the ledger");
      }
    }
    for (const entry of refused) {
      if ((await inbox.processedAt(entry.messageId)) !== 0n)
        fail(entry, "delivered although its Outbox was no longer authorized");
    }
    // Nothing reached the destination that was not an authorized publication on A or B.
    for (const messageId of new Set(executed.map((log) => log.topics[1]))) {
      if (!expected.some((entry) => entry.messageId === messageId))
        failures.push({
          messageId,
          outbox: "?",
          block: 0,
          why: "delivered without an authorized publication on A or B",
        });
    }

    // The default-following publisher moved from A to B exactly when Discovery did.
    for (const publication of follower.confirmed) {
      const wanted = publication.resolvedAt >= defaultChange ? "B" : "A";
      if (publication.target.name !== wanted)
        failures.push({
          messageId: publication.messageId,
          outbox: publication.target.name,
          block: publication.block,
          why: `follower chose ${publication.target.name} at block ${publication.resolvedAt}, expected ${wanted}`,
        });
    }

    const summary = {
      defaultChange,
      removal,
      published: ledger.length,
      authorized: expected.length,
      refused: refused.map(({ messageId, block }) => ({ messageId, block })),
      byOutbox: {
        A: ledger.filter((e) => e.target.name === "A").length,
        B: ledger.filter((e) => e.target.name === "B").length,
      },
      restarts,
      failures,
    };
    await stack.event("rotation-ledger", summary);
    if (failures.length)
      throw new FatalError(
        `outbox rotation lost or misrouted ${failures.length} message(s):\n` +
          failures
            .map(
              (f) =>
                `  ${f.messageId} (Outbox ${f.outbox}, block ${f.block}): ${f.why}`,
            )
            .join("\n"),
      );
    return summary;
  } finally {
    sealer.release();
    provider.destroy();
    destination.destroy();
  }
}
