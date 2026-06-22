/**
 * Shared message types used by both attester and relayer.
 */

/** A message voted on by the attester and forwarded to the relayer for delivery. */
export interface DeliveredMessage {
  /** Unique message ID (bytes32 hex) */
  messageId: string;
  /** Emitter UC address as bytes32 hex (bytes20 address padded to 32 bytes) */
  emitterAddress: string;
  /** Hex-encoded message payload — already abi.encode(address destinationContract, bytes payloadData) */
  payload: string;
  /** Whether this message requires acknowledgment on the source chain after delivery */
  requiresAck: boolean;
  /** ECDSA signatures from attester(s) voting on this message */
  signedVotes: string[];
}
