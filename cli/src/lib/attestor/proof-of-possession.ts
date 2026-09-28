import { decodeAddress } from '@polkadot/util-crypto';

/**
 * Domain-separation tag for the BLS proof-of-possession message.
 *
 * Must stay byte-identical to `POP_DOMAIN_V1` in `primitives/attestor/src/lib.rs`.
 */
const POP_DOMAIN_V1 = 'CC3:ATTESTOR:POP:v1';

/**
 * Build the message a BLS proof of possession must be signed over.
 *
 * This mirrors `proof_of_possession_message` in `primitives/attestor/src/lib.rs`. The runtime
 * binds the proof to the claiming account and the chain so that a proof observed in a public
 * `attest` transaction cannot be replayed by a different controller to seize ownership of a BLS
 * key it does not hold.
 *
 * Layout: `POP_DOMAIN_V1` ++ chainKey as little-endian u64 ++ the account's 32 raw bytes ++ the
 * 48-byte BLS public key. `AccountId32` SCALE-encodes to exactly its 32 raw bytes, which is what
 * `decodeAddress` returns, so the two encodings agree.
 *
 * This is a second implementation of a format the runtime owns, so it can drift. If `attest`
 * starts failing with `InvalidProofOfPossession`, check this function against the Rust one first.
 */
export function proofOfPossessionMessage(
    chainKey: bigint | number,
    attestorAddress: string,
    blsPublicKey: Uint8Array,
): Uint8Array {
    const domain = new TextEncoder().encode(POP_DOMAIN_V1);

    const chainKeyBytes = new Uint8Array(8);
    new DataView(chainKeyBytes.buffer).setBigUint64(0, BigInt(chainKey), true);

    const accountBytes = decodeAddress(attestorAddress);

    const message = new Uint8Array(domain.length + chainKeyBytes.length + accountBytes.length + blsPublicKey.length);
    let offset = 0;
    for (const part of [domain, chainKeyBytes, accountBytes, blsPublicKey]) {
        message.set(part, offset);
        offset += part.length;
    }
    return message;
}
