import { commandSync } from 'execa';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { randomTestAccount } from './helpers';

describe('Show address command', () => {
    it('should return the correct address', async () => {
        await cryptoWaitReady();

        const caller = randomTestAccount();

        const result = commandSync(`node dist/index.js show-address`, {
            env: {
                CC_SECRET: caller.secret,
            },
        });

        expect(result.stdout.split('Account address: ')[1]).toEqual(
            caller.address.toString()
        );
    }, 60000);

    it.each([['using pk', true], ['using mnemonic', false]])('should return the correct Alith address', async (text, usePrivateKey) => {
        await cryptoWaitReady();

        const secret = usePrivateKey
            ? "0x5fb92d6e98884f76de468fa3f6278f8807c48bebc13595d45af5bdc4da702133"
            : "bottom drive obey lake curtain smoke basket hold race lonely fit walk";

        const result = commandSync(`node dist/index.js show-address ${usePrivateKey ? "--use-private-key" : ""}`, {
            env: {
                CC_SECRET: secret,
            },
        });

        expect(result.stdout.split('Account address: ')[1]).toEqual(
            "0xf24FF3a9CF04c71Dbc94D0b566f7A27B94566cac"
        );
    }, 60000);
});