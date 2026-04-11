/**
 * Attester event types.
 * Matches Outbox.MessagePublished event.
 */

/** Parsed from MessagePublished event on Outbox (Source chain) */
export interface PublishedMessage {
  /** Unique message ID (bytes32 hex) */
  messageId: string;
  /** Emitter UC address as bytes32 hex */
  emitterAddress: string;
  /** Whether this message requires acknowledgment */
  requiresAck: boolean;
  /** Hex-encoded message payload */
  payload: string;
}

export interface DeliveredMessage {
  /** Unique message ID (bytes32 hex) */
  messageId: string;
  /** Emitter UC address as bytes32 hex */
  emitterAddress: string;
  /** Whether this message requires acknowledgment */
  requiresAck: boolean;
  /** Hex-encoded message payload */
  payload: string;
  /** Signed votes on the message */
  signedVotes: string[];
}
