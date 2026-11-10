import { AbiCoder, keccak256 } from "ethers";

/**
 * `OutboxTypes.computeMessageId`: the ID an Outbox assigns at publication and the Inbox recomputes
 * at delivery. Lets a check derive the ID from a raw `MessagePublished` log instead of trusting it.
 */
export function computeMessageId(
  outbox: string,
  emitter: string,
  sequence: bigint,
  payload: string,
  sourceChainId: bigint,
): string {
  return keccak256(
    AbiCoder.defaultAbiCoder().encode(
      ["address", "address", "uint64", "bytes32", "uint256"],
      [outbox, emitter, sequence, keccak256(payload), sourceChainId],
    ),
  );
}
