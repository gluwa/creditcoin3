/**
 * simpleDApp-specific event types.
 * Matches simpleDApp.MessageDispatched event.
 */

/** Parsed from MessagePublished event on Outbox (Source chain) */
export interface DispatchedMessage {
  /** Unique message ID (bytes32 hex) */
  messageId: string;
}
