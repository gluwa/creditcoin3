/**
 * Relayer message types.
 * Matches DummyInbox.deliverMessage(messageId, emitterAddress, payload, votes).
 */

/** A "ready" message from mock P2P (2/3+1 votes reached) */
export interface ReadyMessage {
  /** Unique message ID (bytes32 hex) */
  messageId: string;
  /** Emitter address on source chain */
  emitterAddress: string;
  /** Payload: abi.encode(destinationContract, payloadData) */
  destinationContract: string;
  /** Inner payload to pass to receiveMessage */
  payloadData: string;
  /** Pre-signed votes (dummy validator accepts empty) */
  votes?: string;
}
