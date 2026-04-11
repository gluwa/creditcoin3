/**
 * Attester event types.
 * Matches Outbox.MessagePublished and Inbox.MessageDelivered events.
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

/** Parsed from MessageDelivered event on Inbox (Destination chain) */
export interface DeliveredMessage {
  /** Unique message ID (bytes32 hex) */
  messageId: string;
  /** Address that processed the message */
  processor: string;
}
