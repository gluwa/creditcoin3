import assert from "node:assert/strict";
import test from "node:test";
import { Interface, Wallet } from "ethers";
import { ANVIL_KEY, SOURCE_KEY, type Stack } from "./stack.mjs";
import {
  PUBLICATION_TOPIC,
  assertNoPublicationReplay,
  assertSettled,
  validateDurableState,
  type Publication,
} from "./scenarios.mjs";
import { waitFor } from "./runtime.mjs";

const publication: Publication = {
  messageId: `0x${"11".repeat(32)}`,
  block: 10,
  transactionHash: `0x${"22".repeat(32)}`,
  outbox: `0x${"33".repeat(20)}`,
  payload: "0xbeef",
  emitter: new Wallet(SOURCE_KEY).address,
};
const checkpoint = { "outboxes:2": { block: 14, durable_work: true } };
const journal = {
  message_id: publication.messageId,
  chain_key: 2,
  block_height: 10,
  outbox: publication.outbox,
  tx_hash: publication.transactionHash,
};

test("crash proof rejects lookback replay, legacy checkpoints and unrelated journal records", () => {
  assert.equal(validateDurableState(checkpoint, journal, publication, 2), 14);
  for (const bad of [
    { "outboxes:2": 14 },
    { "outbox:2": { block: 14, durable_work: true } },
    { "outboxes:2": { block: 14, durable_work: false } },
    { "outboxes:2": { block: 12, durable_work: true } },
  ]) {
    assert.throws(() => validateDurableState(bad, journal, publication, 2));
  }
  for (const bad of [
    { ...journal, message_id: `0x${"44".repeat(32)}` },
    { ...journal, tx_hash: `0x${"55".repeat(32)}` },
    { ...journal, chain_key: 3 },
    { ...journal, block_height: 11 },
  ]) {
    assert.throws(
      () => validateDurableState(checkpoint, bad, publication, 2),
      /exact publication/,
    );
  }
});

test("restart proof forbids source reindexing but allows exact-ID ack route lookup", () => {
  const valid = [
    {
      phase: "restarted",
      filter: { topics: [PUBLICATION_TOPIC], fromBlock: "0xd" },
    },
    {
      phase: "restarted",
      filter: {
        topics: [PUBLICATION_TOPIC, publication.messageId],
        fromBlock: "0x0",
      },
    },
  ];
  assert.doesNotThrow(() => assertNoPublicationReplay(valid, publication));
  assert.throws(
    () => assertNoPublicationReplay([], publication),
    /no restarted source scan/,
  );
  assert.throws(
    () =>
      assertNoPublicationReplay(
        [
          ...valid,
          {
            phase: "restarted",
            filter: { topics: [PUBLICATION_TOPIC], fromBlock: "0xa" },
          },
        ],
        publication,
      ),
    /replayed/,
  );
});

test("execution evidence rejects a wrong message, destination, payload or failed receipt", async () => {
  const address = `0x${"66".repeat(20)}`;
  const dapp = `0x${"77".repeat(20)}`;
  const abi = new Interface([
    "function processedAt(bytes32) view returns(uint256)",
    "event MessageExecuted(bytes32 indexed messageId,address indexed emitterAddress,address indexed destination,address dispatcher,address relayer,bytes messagePayload)",
  ]);
  for (const defect of ["message", "destination", "payload", "receipt"]) {
    const encoded = abi.encodeEventLog(abi.getEvent("MessageExecuted")!, [
      defect === "message" ? `0x${"88".repeat(32)}` : publication.messageId,
      publication.emitter,
      defect === "destination" ? address : dapp,
      address,
      new Wallet(ANVIL_KEY).address,
      defect === "payload" ? "0xdead" : publication.payload,
    ]);
    const log = {
      ...encoded,
      address,
      blockNumber: "0x14",
      blockHash: `0x${"99".repeat(32)}`,
      transactionHash: `0x${"aa".repeat(32)}`,
    };
    const fixture = {
      deployment: { dest: { inbox: address, dapp } },
      destination: {
        call: async (method: string, params: unknown[]) => {
          if (method === "eth_call") {
            assert.equal(
              abi.parseTransaction(params[0] as { data: string })!.args[0],
              publication.messageId,
            );
            return abi.encodeFunctionResult("processedAt", [20]);
          }
          if (method === "eth_getLogs") return [log];
          if (method === "eth_getTransactionReceipt")
            return {
              ...log,
              status: defect === "receipt" ? "0x0" : "0x1",
              logs: [log],
            };
          throw new Error(`unexpected ${method}`);
        },
      },
      wait: (label: string, probe: () => Promise<unknown>) =>
        waitFor(label, probe, { timeoutMs: 100 }),
    };
    await assert.rejects(
      assertSettled(fixture as unknown as Stack, publication),
      /another contract|does not match|successful transaction/,
    );
  }
});
