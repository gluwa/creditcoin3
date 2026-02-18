/**
 * Delivers a ready message to the DummyInbox contract.
 */

import { ethers } from "ethers";
import type { ReadyMessage } from "./types.js";

const INBOX_ABI = [
  "function deliverMessage(bytes32 messageId, address emitterAddress, bytes calldata payload, bytes calldata votes) external returns (bool)",
];

export async function deliverMessage(
  provider: ethers.Provider,
  signer: ethers.Signer,
  inboxAddress: string,
  msg: ReadyMessage
): Promise<{ success: boolean; txHash?: string; error?: string }> {
  const inbox = new ethers.Contract(inboxAddress, INBOX_ABI, signer);

  const messageId = ethers.hexlify(ethers.getBytes(msg.messageId.startsWith("0x") ? msg.messageId : `0x${msg.messageId}`));
  if (messageId.length !== 66) {
    return { success: false, error: "messageId must be 32 bytes (64 hex chars)" };
  }

  const payload = ethers.AbiCoder.defaultAbiCoder().encode(
    ["address", "bytes"],
    [msg.destinationContract, msg.payloadData.startsWith("0x") ? msg.payloadData : `0x${msg.payloadData}`]
  );
  const votes = msg.votes ?? "0x";

  try {
    const tx = await inbox.deliverMessage(
      messageId,
      msg.emitterAddress,
      payload,
      votes,
      { gasLimit: 500_000 }
    );
    const receipt = await tx.wait();
    return { success: true, txHash: receipt!.hash };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { success: false, error: message };
  }
}
