import { encodeAddress } from '@polkadot/util-crypto';
import { u8aToHex } from '@polkadot/util';
import { proofOfPossessionMessage } from '../../lib/attestor/proof-of-possession';

describe('proofOfPossessionMessage()', () => {
    // Byte-for-byte the vector pinned by `message_layout_is_pinned` in
    // `primitives/attestor/src/lib.rs`. The runtime owns this format; this file is a second
    // implementation of it, so the two can drift and the symptom would be every attestor failing
    // to register with InvalidProofOfPossession. If this test breaks, reconcile with the Rust one.
    const EXPECTED =
        '0x4343333a4154544553544f523a504f503a7631' + // "CC3:ATTESTOR:POP:v1"
        '0200000000000000' + // chain key 2, little-endian u64
        '11'.repeat(32) + // attestor account, 32 raw bytes
        '22'.repeat(48); // BLS public key, 48 bytes

    it('matches the runtime vector', () => {
        const address = encodeAddress(new Uint8Array(32).fill(0x11));
        const blsPublicKey = new Uint8Array(48).fill(0x22);

        const message = proofOfPossessionMessage(2, address, blsPublicKey);

        expect(message.length).toEqual(19 + 8 + 32 + 48);
        expect(u8aToHex(message)).toEqual(EXPECTED);
    });
});
