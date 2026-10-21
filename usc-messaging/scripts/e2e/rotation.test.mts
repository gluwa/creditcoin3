import assert from "node:assert/strict";
import test from "node:test";
import { AbiCoder, Interface, Wallet } from "ethers";
import { committee, updateDigest, verifySetUpdate } from "./rotation.mjs";

const validator = `0x${"ab".repeat(20)}`;
const otherValidator = `0x${"cd".repeat(20)}`;
const initial = committee.map((key) => key.address);
const current = initial.slice(0, 3);
const abi = AbiCoder.defaultAbiCoder();
const contract = new Interface([
  "function submitAttestorSetUpdate(address[],bytes)",
]);

function proposal(signers = committee.slice(0, 3), next = initial) {
  const digest = updateDigest(validator, next, 31337n, 8n);
  return contract.encodeFunctionData("submitAttestorSetUpdate", [
    next,
    abi.encode(
      ["bytes[]"],
      [signers.map((key) => key.signingKey.sign(digest).serialized)],
    ),
  ]);
}

test("rotation evidence verifies the exact fresh epoch and all surviving signers", () => {
  const result = verifySetUpdate(
    proposal(),
    validator,
    31337n,
    8n,
    current,
    initial,
    3,
  );
  assert.deepEqual(result.signers, current);
  assert.deepEqual(result.next, initial);
  assert.equal(result.nonce, "8");
  // Contract allows any member ordering, but the signatures must bind that exact ordering.
  const reversed = [...initial].reverse();
  assert.deepEqual(
    verifySetUpdate(
      proposal(committee.slice(0, 3), reversed),
      validator,
      31337n,
      8n,
      current,
      initial,
      3,
    ).next,
    reversed,
  );
});

test("rotation evidence rejects stale epochs, another chain and another validator", () => {
  for (const [address, chainId, nonce] of [
    [validator, 31337n, 7n],
    [validator, 42n, 8n],
    [otherValidator, 31337n, 8n],
  ] as const) {
    assert.throws(
      () =>
        verifySetUpdate(
          proposal(),
          address,
          chainId,
          nonce,
          current,
          initial,
          3,
        ),
      /stale epoch or unauthorized/,
    );
  }
});

test("rotation evidence rejects revoked, duplicate and insufficient signers", () => {
  const outsider = new Wallet(`0x${"12".repeat(32)}`);
  for (const invalid of [
    [committee[0], committee[1], committee[3]],
    [committee[0], committee[1], outsider],
  ]) {
    assert.throws(
      () =>
        verifySetUpdate(
          proposal(invalid),
          validator,
          31337n,
          8n,
          current,
          initial,
          3,
        ),
      /unauthorized signer/,
    );
  }
  assert.throws(
    () =>
      verifySetUpdate(
        proposal([committee[0], committee[0], committee[1]]),
        validator,
        31337n,
        8n,
        current,
        initial,
        3,
      ),
    /duplicates/,
  );
  assert.throws(
    () =>
      verifySetUpdate(
        proposal(committee.slice(0, 2)),
        validator,
        31337n,
        8n,
        current,
        initial,
        3,
      ),
    /quorum/,
  );
});

test("rotation evidence rejects another replacement set and malformed transaction input", () => {
  assert.throws(
    () =>
      verifySetUpdate(
        proposal(committee.slice(0, 3), current),
        validator,
        31337n,
        8n,
        current,
        initial,
        3,
      ),
    /unexpected membership/,
  );
  assert.throws(() =>
    verifySetUpdate("0x1234", validator, 31337n, 8n, current, initial, 3),
  );
});
