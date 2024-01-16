import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise } from '../../lib';

describe('Events that WILL brick the blockchain', (): void => {
    let api: ApiPromise;

    beforeAll(async () => {
        api = (await newApi((global as any).CREDITCOIN_API_URL)).api;
    });

    afterAll(async () => {
        await api.disconnect();
    });

    test('EPOCH_DURATION has changed', () => {
        let expectedValue = 2880;

        if ((global as any).CREDITCOIN_USES_FAST_RUNTIME === true) {
            expectedValue = 15;
        }

        const epochDuration = (api.consts.babe.epochDuration as U64).toNumber();
        expect(epochDuration).toEqual(expectedValue);
    });

    test('Block time has changed', () => {
        let expectedValue = 15000;

        if ((global as any).CREDITCOIN_USES_FAST_RUNTIME === true) {
            expectedValue = 5000;
        }

        const blockTime = (api.consts.babe.expectedBlockTime as U64).toNumber();
        expect(blockTime).toEqual(expectedValue);
    });

    test('Minimum period has changed', () => {
        // blockTime / 2
        let expectedValue = 7500;

        if ((global as any).CREDITCOIN_USES_FAST_RUNTIME === true) {
            expectedValue = 2500;
        }

        const blockTime = (api.consts.timestamp.minimumPeriod as U64).toNumber();
        expect(blockTime).toEqual(expectedValue);
    });
});
