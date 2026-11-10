import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { getAddress } from "ethers";
import { decodeEvmEnvelope, encodeEvmEnvelope } from "../evm-envelope.mjs";
import { computeMessageId } from "./message-id.mjs";
import { updateDigest } from "./rotation.mjs";

// The golden vectors asc-contracts generates from Solidity. `common/write-ability` holds this
// repository's copy and guards it against the canonical files byte for byte.
function vectors<T>(name: string): T[] {
  const entries = JSON.parse(
    readFileSync(
      new URL(`../../../common/write-ability/vectors/${name}`, import.meta.url),
      "utf8",
    ),
  ) as T[];
  assert.ok(entries.length > 0, `${name} is empty`);
  return entries;
}

test("message IDs match the Solidity vectors", () => {
  for (const v of vectors<{
    name: string;
    outbox: string;
    emitter: string;
    sequence: string;
    payload: string;
    sourceChainId: string;
    messageId: string;
  }>("message_id.json")) {
    assert.equal(
      computeMessageId(
        v.outbox,
        v.emitter,
        BigInt(v.sequence),
        v.payload,
        BigInt(v.sourceChainId),
      ),
      v.messageId,
      v.name,
    );
  }
});

test("EVM delivery envelopes match the Solidity vectors", () => {
  for (const v of vectors<{
    name: string;
    destinationContractAddress: string;
    nativeCoinValue: string;
    gasLimit: string;
    payloadData: string;
    encoded: string;
  }>("evm_envelope.json")) {
    const decoded = decodeEvmEnvelope(v.encoded);
    assert.deepEqual(
      {
        destination: decoded.destination,
        nativeCoinValue: decoded.nativeCoinValue,
        gasLimit: decoded.gasLimit,
        payloadData: decoded.payloadData,
      },
      {
        destination: getAddress(v.destinationContractAddress),
        nativeCoinValue: BigInt(v.nativeCoinValue),
        gasLimit: BigInt(v.gasLimit),
        payloadData: v.payloadData,
      },
      v.name,
    );
    const options = {
      nativeCoinValue: BigInt(v.nativeCoinValue),
      gasLimit: BigInt(v.gasLimit),
    };
    // The publisher-side encoder refuses a zero gas limit, which DispatcherRouter would reject.
    if (options.gasLimit === 0n) {
      assert.throws(() =>
        encodeEvmEnvelope(v.destinationContractAddress, v.payloadData, options),
      );
    } else {
      assert.equal(
        encodeEvmEnvelope(v.destinationContractAddress, v.payloadData, options),
        v.encoded,
        v.name,
      );
    }
  }
});

test("attestor set-update digests match the Solidity vectors", () => {
  for (const v of vectors<{
    name: string;
    validator: string;
    attestors: string[];
    chainId: string;
    nonce: string;
    digest: string;
    canonicalAttestors: string[];
    canonicalDigest: string;
  }>("attestor_set_update.json")) {
    const [chainId, nonce] = [BigInt(v.chainId), BigInt(v.nonce)];
    assert.equal(
      updateDigest(v.validator, v.attestors, chainId, nonce),
      v.digest,
      v.name,
    );
    assert.equal(
      updateDigest(v.validator, v.canonicalAttestors, chainId, nonce),
      v.canonicalDigest,
      v.name,
    );
  }
});
