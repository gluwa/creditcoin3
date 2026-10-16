// EVM delivery envelope — asc-contracts #36 (main a9791c37).
//
// Since #36 the destination Inbox no longer calls `IMessageReceiver.receiveMessage` on a fixed
// dApp. Its `messageDispatcher` is the `DispatcherRouter`, which decodes the Outbox payload as
//
//   abi.encode(address destination, uint256 nativeCoinValue, uint256 gasLimit, bytes payloadData)
//
// (contracts/write-ability/common/EVMPayloadCodec.sol) and calls `destination` with raw calldata
// `payloadData ++ bytes20(emitterAddress)`, forwarding `gasLimit` gas and `nativeCoinValue` wei.
// `gasLimit == 0` reverts `InvalidGasLimit`. Attestors sign — and the quoter's `payloadHash`
// must hash — the FULL envelope bytes: that is what the Outbox stores and the relayer forwards.
import { ethers } from "ethers";

/// Destination-side gas budget the dispatcher forwards to `destination`. MockDestination's
/// fallback only bumps a counter, so this is generous; real dApps size it to their handler.
export const DEFAULT_ENVELOPE_GAS_LIMIT = 200_000n;

export interface EnvelopeOptions {
  /// Wei the dispatcher forwards with the destination call (default 0 — the relayer currently
  /// refuses nonzero values, MAX_NATIVE_COIN_VALUE_WEI = 0).
  nativeCoinValue?: bigint;
  /// Gas forwarded to the destination call; must be > 0.
  gasLimit?: bigint;
}

/// `abi.encode(destination, nativeCoinValue, gasLimit, payloadData)` as 0x-hex.
export function encodeEvmEnvelope(
  destination: string,
  payloadData: Uint8Array | string,
  { nativeCoinValue = 0n, gasLimit = DEFAULT_ENVELOPE_GAS_LIMIT }: EnvelopeOptions = {},
): string {
  if (gasLimit <= 0n) throw new Error("envelope gasLimit must be > 0 (DispatcherRouter rejects 0)");
  return ethers.AbiCoder.defaultAbiCoder().encode(
    ["address", "uint256", "uint256", "bytes"],
    [ethers.getAddress(destination), nativeCoinValue, gasLimit, payloadData],
  );
}

/// Inverse of `encodeEvmEnvelope`, for logging / assertions.
export function decodeEvmEnvelope(envelope: string | Uint8Array): {
  destination: string;
  nativeCoinValue: bigint;
  gasLimit: bigint;
  payloadData: string;
} {
  const [destination, nativeCoinValue, gasLimit, payloadData] = ethers.AbiCoder.defaultAbiCoder().decode(
    ["address", "uint256", "uint256", "bytes"],
    envelope,
  );
  return { destination, nativeCoinValue, gasLimit, payloadData };
}

/// The memo-style envelope every publisher script here sends: utf8 memo to `destination`, no
/// native value. Reads `ENVELOPE_GAS_LIMIT` so an operator can widen the destination budget.
export function memoEnvelope(destination: string, memo: string): string {
  const gasLimit = process.env.ENVELOPE_GAS_LIMIT ? BigInt(process.env.ENVELOPE_GAS_LIMIT) : DEFAULT_ENVELOPE_GAS_LIMIT;
  return encodeEvmEnvelope(destination, ethers.toUtf8Bytes(memo), { nativeCoinValue: 0n, gasLimit });
}
