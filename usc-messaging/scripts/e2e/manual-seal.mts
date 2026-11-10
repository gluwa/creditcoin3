import { FatalError } from "./runtime.mjs";
import { assertSettled, publish } from "./scenarios.mjs";
import type { Stack } from "./stack.mjs";

/** OutboxDiscovery.MIN_TIMELOCK: the shortest window a default or removal change can take. */
export const DISCOVERY_TIMELOCK_BLOCKS = 1200;

/**
 * Proves the full stack survives a paced jump across a Discovery timelock: a message settles
 * before the jump and another settles after it, with the same attestors, proof generator and
 * relayer processes.
 */
export async function manualSealSmoke(stack: Stack): Promise<unknown> {
  if (!stack.sealer)
    throw new FatalError("manual-seal-smoke needs a manual-seal stack");
  const before = await assertSettled(
    stack,
    await publish(stack, "seal-before"),
  );
  const from = await stack.sealer.height();
  const started = Date.now();
  const reached = await stack.sealer.fastForward(
    from + DISCOVERY_TIMELOCK_BLOCKS,
    Number(process.env.E2E_FAST_FORWARD_BLOCKS_PER_SECOND ?? 10),
  );
  const jump = { from, reached, seconds: (Date.now() - started) / 1000 };
  await stack.event("fast-forward", jump);
  const after = await assertSettled(stack, await publish(stack, "seal-after"));
  return { before, jump, after };
}
