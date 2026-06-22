/**
 * Attester-specific event types.
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

export type { DeliveredMessage } from "../types.js";
