import {
  AbiCoder,
  Contract,
  Interface,
  JsonRpcProvider,
  Wallet,
  getAddress,
  isError,
  keccak256,
  recoverAddress,
} from "ethers";
import { ANVIL_KEY, type Stack } from "./stack.mjs";
import { committee } from "./committee.mjs";
import { assertSettled, publish, requireEvidence } from "./scenarios.mjs";

const abi = AbiCoder.defaultAbiCoder();
const validatorAbi = new Interface([
  "function attestorSetUpdateNonce() view returns(uint256)",
  "function attestors() view returns(address[])",
  "function threshold() view returns(uint256)",
  "function submitAttestorSetUpdate(address[] newAttestors,bytes signatures)",
  "event AttestorSetUpdated(address[] newAttestors)",
  "error InvalidAttestor()",
]);
const registryAbi = [
  "function setUpdater(address,bool)",
  "function removeAttestor(address)",
  "function membershipNonce() view returns(uint256)",
];

const normalized = (addresses: readonly string[]) =>
  addresses.map(getAddress).sort();
const sameSet = (a: readonly string[], b: readonly string[]) =>
  JSON.stringify(normalized(a)) === JSON.stringify(normalized(b));

export function updateDigest(
  validator: string,
  next: readonly string[],
  chainId: bigint,
  nonce: bigint,
): string {
  return keccak256(
    abi.encode(
      ["address", "address[]", "uint256", "uint256"],
      [validator, next, chainId, nonce],
    ),
  );
}

/** Independently verify the fresh proposal actually mined by the real relayer. */
export function verifySetUpdate(
  data: string,
  validator: string,
  chainId: bigint,
  nonce: bigint,
  current: readonly string[],
  expected: readonly string[],
  quorum: number,
) {
  const decoded = validatorAbi.decodeFunctionData(
    "submitAttestorSetUpdate",
    data,
  );
  const next = Array.from(decoded.newAttestors as string[]);
  requireEvidence(
    sameSet(next, expected),
    "set-update transaction proposes unexpected membership",
  );
  const digest = updateDigest(validator, next, chainId, nonce);
  const [signatures] = abi.decode(["bytes[]"], decoded.signatures);
  const signers = Array.from(signatures as string[]).map((signature) =>
    recoverAddress(digest, signature),
  );
  const allowed = new Set(current.map(getAddress));
  requireEvidence(
    signers.every((signer) => allowed.has(signer)),
    "set-update signature has a stale epoch or unauthorized signer",
  );
  requireEvidence(
    new Set(signers).size === signers.length,
    "set-update signatures contain duplicates",
  );
  requireEvidence(
    signers.length >= quorum,
    "set-update signatures do not reach the surviving committee quorum",
  );
  return { nonce: nonce.toString(), digest, signers, next };
}

export async function attestorRotation(stack: Stack): Promise<unknown> {
  const provider = new JsonRpcProvider(stack.destination.url, 31337, {
    staticNetwork: true,
    cacheTimeout: -1,
  });
  provider.pollingInterval = 500;
  const owner = new Wallet(ANVIL_KEY, provider);
  const address = stack.deployment.dest.voteValidator;
  const validator = new Contract(address, validatorAbi, owner);
  const registry = new Contract(
    stack.deployment.dest.attestorRegistry,
    registryAbi,
    owner,
  );
  try {
    const initial = Array.from((await validator.attestors()) as string[]);
    const expected = committee.map((key) => key.address);
    requireEvidence(
      sameSet(initial, expected),
      "live attestors do not match the four development identities",
    );
    const next = normalized(initial);
    const survivors = committee.slice(0, 3);
    const revoked = committee[3].address;
    const before = (await validator.attestorSetUpdateNonce()) as bigint;
    const membershipBefore = (await registry.membershipNonce()) as bigint;
    const digest = updateDigest(address, next, 31337n, before);
    const signatures = abi.encode(
      ["bytes[]"],
      [survivors.map((key) => key.signingKey.sign(digest).serialized)],
    );
    const staleData = validatorAbi.encodeFunctionData(
      "submitAttestorSetUpdate",
      [next, signatures],
    );
    // Prove this bundle is valid before revocation. All its signers survive the change.
    await provider.call({ to: address, from: owner.address, data: staleData });

    // Hold voted replacements while delivery is tested against the reduced committee. Message
    // validation remains enabled; attestors and the relayer continue running throughout.
    await (await registry.setUpdater(address, false)).wait();
    const removal = await (await registry.removeAttestor(revoked)).wait();
    requireEvidence(
      removal?.status === 1,
      "revocation has no successful receipt",
    );
    const assertRevoked = async () => {
      requireEvidence(
        sameSet(
          await validator.attestors(),
          survivors.map((key) => key.address),
        ),
        "revoked attestor is still in the destination committee",
      );
      requireEvidence(
        (await validator.attestorSetUpdateNonce()) === before + 1n &&
          (await registry.membershipNonce()) === membershipBefore + 1n,
        "revocation did not advance the shared epoch exactly once",
      );
    };
    await assertRevoked();
    const rejectStale = async () => {
      let revert: string | null = null;
      try {
        await provider.call({
          to: address,
          from: owner.address,
          data: staleData,
        });
      } catch (error) {
        if (isError(error, "CALL_EXCEPTION")) revert = error.data;
        else throw error;
      }
      requireEvidence(
        revert === validatorAbi.getError("InvalidAttestor")!.selector,
        "stale approval was accepted or rejected for an unrelated reason",
      );
    };
    await rejectStale();
    // Send with explicit gas so estimation cannot replace the actual mined failure assertion.
    const failed = await owner.sendTransaction({
      to: address,
      data: staleData,
      gasLimit: 1_000_000,
    });
    const failedReceipt = await stack.wait(
      "mined stale-approval rejection",
      async () => {
        const receipt = await provider.getTransactionReceipt(failed.hash);
        return receipt ?? undefined;
      },
    );
    requireEvidence(
      failedReceipt.status === 0,
      "stale approval transaction unexpectedly succeeded",
    );
    await assertRevoked();
    const whileRevoked = await assertSettled(
      stack,
      await publish(stack, "attestor-revoked"),
    );
    await assertRevoked();

    // Only the real attestors and relayer submit the fresh proposal. The elected source set
    // still contains all four identities, so its proposer reconciles the destination set.
    const quorum = Number(await validator.threshold());
    requireEvidence(
      quorum === 3,
      "surviving committee must require all three live signers",
    );
    const enabled = await (await registry.setUpdater(address, true)).wait();
    requireEvidence(
      enabled?.status === 1,
      "updater authorization has no successful receipt",
    );
    const rotation = await stack.wait(
      "fresh attestor-voted rotation through the actual relayer",
      async () => {
        const nonce = (await validator.attestorSetUpdateNonce()) as bigint;
        if (nonce === before + 1n) return undefined;
        requireEvidence(
          nonce === before + 2n &&
            (await registry.membershipNonce()) === membershipBefore + 2n,
          "fresh rotation advanced the epoch unexpectedly",
        );
        requireEvidence(
          sameSet(await validator.attestors(), initial),
          "fresh rotation did not restore the elected committee",
        );
        const events = await provider.getLogs({
          address,
          topics: [validatorAbi.getEvent("AttestorSetUpdated")!.topicHash],
          fromBlock: enabled.blockNumber,
        });
        requireEvidence(
          events.length === 1,
          "rotation must have exactly one validator update event",
        );
        const receipt = await provider.getTransactionReceipt(
          events[0].transactionHash,
        );
        const transaction = await provider.getTransaction(
          events[0].transactionHash,
        );
        requireEvidence(
          receipt?.status === 1 &&
            transaction &&
            !events[0].removed &&
            receipt.blockHash === events[0].blockHash,
          "fresh rotation has no canonical successful transaction receipt",
        );
        requireEvidence(
          transaction.to === getAddress(address) &&
            transaction.from === owner.address,
          "fresh rotation has the wrong target or relayer sender",
        );
        const proposal = verifySetUpdate(
          transaction.data,
          address,
          31337n,
          before + 1n,
          survivors.map((key) => key.address),
          initial,
          quorum,
        );
        return {
          transactionHash: transaction.hash,
          block: receipt.blockNumber,
          ...proposal,
        };
      },
      240_000,
    );
    await rejectStale();
    const afterRotation = await assertSettled(
      stack,
      await publish(stack, "attestor-rotated"),
    );
    const evidence = {
      validator: address,
      registry: stack.deployment.dest.attestorRegistry,
      revoked,
      initialNonce: before.toString(),
      revocationTransaction: removal.hash,
      staleApproval: {
        nonce: before.toString(),
        digest,
        transactionHash: failed.hash,
        status: failedReceipt.status,
      },
      whileRevoked,
      rotation,
      afterRotation,
    };
    await stack.event("attestor-rotation", evidence);
    return evidence;
  } finally {
    provider.destroy();
  }
}
