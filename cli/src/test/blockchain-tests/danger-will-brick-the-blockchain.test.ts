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
        const epochDuration = (api.consts.babe.epochDuration as U64).toNumber();
        expect(epochDuration).toEqual((global as any).CREDITCOIN_EXPECTED_EPOCH_DURATION);
    });

    test('Block time has changed', () => {
        const blockTime = (api.consts.babe.expectedBlockTime as U64).toNumber();
        expect(blockTime).toEqual((global as any).CREDITCOIN_EXPECTED_BLOCK_TIME);
    });

    test('Minimum period has changed', () => {
        const minPeriod = (api.consts.timestamp.minimumPeriod as U64).toNumber();
        // expected is blockTime / 2
        expect(minPeriod).toEqual((global as any).CREDITCOIN_EXPECTED_MINIMUM_PERIOD);
    });
});
