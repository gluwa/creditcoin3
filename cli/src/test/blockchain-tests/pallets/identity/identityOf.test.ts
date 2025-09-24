import { newApi, ApiPromise, KeyringPair } from '../../../../lib';
import { u8aToHex } from '../../../../lib/common';
import { forElapsedBlocks } from '../../../utils';

describe('identityOf', (): void => {
    let alice: KeyringPair;
    let api: ApiPromise;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
        const identity = {
            display: {
                raw: 'TestingAccount',
            },
            legal: {
                raw: 'Gluwa',
            },
            email: {
                raw: 'testing@gluwa.com',
            },
            twitter: {
                raw: 'gluwa-bot',
            },
        };

        const nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.identity.setIdentity(identity).signAndSend(alice, { nonce });

        await forElapsedBlocks(api, { minBlocks: 2 });
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('should be able to decode storage query', async () => {
        const identity = await api.query.identity.identityOf(alice.address);
        // note: can't figure out this data type mapping properly
        const asString = identity.toString();
        console.log(`***** IDENTITY=${asString}`);

        expect(asString).toContain(u8aToHex(new TextEncoder().encode('TestingAccount')));
        expect(asString).toContain(u8aToHex(new TextEncoder().encode('Gluwa')));
        expect(asString).toContain(u8aToHex(new TextEncoder().encode('testing@gluwa.com')));
        expect(asString).toContain(u8aToHex(new TextEncoder().encode('gluwa-bot')));
    });
});
