import { commandSync } from 'execa';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { randomTestAccount } from './helpers';

describe('Show address command', () => {
    it('should return the correct address when %s', async () => {
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
});
