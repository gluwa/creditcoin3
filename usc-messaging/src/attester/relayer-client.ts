/**
 * HTTP client for notifying the relayer about voted messages.
 * For the POC this is a best-effort call — failures are logged but not fatal.
 */

import type { DeliveredMessage } from "../types.js";

/**
 * POSTs a voted message to the relayer's /deliver endpoint.
 */
export async function notifyRelayer(
  relayerUrl: string,
  message: DeliveredMessage,
): Promise<void> {
  const url = `${relayerUrl}/deliver`;
  const body = {
    messageId: message.messageId,
    emitterAddress: message.emitterAddress,
    payload: message.payload,
    requiresAck: message.requiresAck,
    signedVotes: message.signedVotes,
  };

  console.log(`[Relayer] POST ${url} messageId=${message.messageId}`);

  try {
    const res = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });

    if (res.ok) {
      console.log(
        `[Relayer] messageId=${message.messageId} successfully notified to relayer`,
      );
    } else {
      const text = await res.text();
      console.warn(`[Relayer] HTTP ${res.status}: ${text}`);
    }
  } catch (err) {
    console.warn(
      `[Relayer] Failed to reach relayer at ${url}:`,
      err instanceof Error ? err.message : err,
    );
  }
}
