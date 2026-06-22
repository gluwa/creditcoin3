/**
 * Delivers a ready message to the SimpleInbox contract.
 */

import { ethers } from "ethers";
import type { DeliveredMessage } from "./types.js";

const INBOX_ABI = [
  "function deliverMessage(bytes32 messageId, address emitterAddress, bytes calldata payload, bytes calldata votes) external returns (bool)",
];

export async function deliverMessage(
  signer: ethers.Signer,
  inboxAddress: string,
  msg: DeliveredMessage,
): Promise<{ success: boolean; txHash?: string; error?: string }> {
  const inbox = new ethers.Contract(inboxAddress, INBOX_ABI, signer);

  const messageId = ethers.hexlify(
    ethers.getBytes(
      msg.messageId.startsWith("0x") ? msg.messageId : `0x${msg.messageId}`,
    ),
  );
  if (messageId.length !== 66) {
    return {
      success: false,
      error: "messageId must be 32 bytes (64 hex chars)",
    };
  }

  // emitterAddress arrives as a bytes32 hex (bytes20 address padded to 32 bytes).
  // SimpleInbox.deliverMessage expects address — extract the last 20 bytes.
  const emitterAddress = ethers.getAddress(
    "0x" + msg.emitterAddress.replace("0x", "").slice(-40),
  );

  // payload is already abi.encode(address destinationContract, bytes payloadData) — pass through as-is.
  const payload = msg.payload.startsWith("0x")
    ? msg.payload
    : `0x${msg.payload}`;

  // Encode the array of ECDSA signatures into bytes for the vote validator.
  const votes = ethers.AbiCoder.defaultAbiCoder().encode(
    ["bytes[]"],
    [msg.signedVotes],
  );

  try {
    const tx = await inbox.deliverMessage(
      messageId,
      emitterAddress,
      payload,
      votes,
      { gasLimit: 500_000 },
    );
    const receipt = await tx.wait();
    return { success: true, txHash: receipt!.hash };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    return { success: false, error: message };
  }
}
