import assert from "node:assert/strict";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import type { Rpc } from "./rpc.mjs";
import { Sealer } from "./sealer.mjs";

/** A chain that grows only when asked to, like a `--sealing manual` node. */
function chain(start: number) {
  const state = { height: start, finalized: start };
  const rpc = {
    async call(method: string, params: unknown[] = []) {
      if (method === "eth_blockNumber") return `0x${state.height.toString(16)}`;
      if (method === "engine_createBlock") {
        state.height += 1;
        return { hash: `0x${state.height.toString(16)}` };
      }
      if (method === "engine_finalizeBlock") {
        state.finalized = Number(BigInt(params[0] as string));
        return true;
      }
      throw new Error(`unexpected ${method}`);
    },
  } as unknown as Rpc;
  return { state, rpc };
}

test("the sealer stops exactly at a held block and seals one block at a time from there", async () => {
  const { state, rpc } = chain(100);
  const sealer = new Sealer(rpc, 5, new AbortController().signal);
  sealer.start();
  try {
    await sealer.holdAt(140, 1000);
    assert.equal(state.height, 140);
    await delay(50);
    assert.equal(state.height, 140, "a held sealer must not seal on its own");

    assert.equal(await sealer.sealOne(), 141);
    assert.equal(await sealer.sealOne(), 142);
    assert.deepEqual(state, { height: 142, finalized: 142 });
    await delay(50);
    assert.equal(state.height, 142);

    await assert.rejects(sealer.holdAt(120, 1000), /already past block 120/);

    sealer.release();
    assert.ok((await sealer.fastForward(160, 1000)) >= 160);

    const paused = await sealer.pause();
    await delay(50);
    assert.equal(state.height, paused, "a paused sealer must not seal");
    assert.equal(await sealer.sealOne(), paused + 1);
  } finally {
    await sealer.stop();
  }
});

test("sealing one block needs a held sealer, and a failing node stops every waiter", async () => {
  const { rpc } = chain(10);
  const sealer = new Sealer(rpc, 5, new AbortController().signal);
  sealer.start();
  await assert.rejects(sealer.sealOne(), /needs a held sealer/);
  await sealer.stop();

  const broken = {
    async call(method: string) {
      if (method === "eth_blockNumber") return "0x1";
      throw new Error("node is gone");
    },
  } as unknown as Rpc;
  const failing = new Sealer(broken, 5, new AbortController().signal);
  failing.start();
  await delay(50);
  assert.throws(() => failing.assertAlive(), /manual sealer stopped/);
  await assert.rejects(failing.holdAt(5, 1000), /manual sealer stopped/);
  await failing.stop();
});
