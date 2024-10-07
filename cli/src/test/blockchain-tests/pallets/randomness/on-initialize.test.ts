import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise } from '../../../../lib';
import { getChainStatus } from '../../../../lib/chain/status';

describe('StoreRandomnessForEpoch events', (): void => {
    let api: ApiPromise;
    const maxBlocks = 70; // > 5 min

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
    });

    afterAll(async () => {
        await api.disconnect();
    });

    test('are emitted frequently and randomness can be queried from storage', async (): Promise<void> => {
        let recordedEvents = 0;
        const initialBlock = (await getChainStatus(api)).bestNumber;

        return new Promise((resolve, reject): void => {
            // Subscribe to system events via storage
            api.query.system
                .events(async (events) => {
                    console.log(`Received ${events.length} events`);

                    // Loop through the Vec<EventRecord>
                    for (const record of events) {
                        const { event, phase: _ } = record;

                        if (`${event.section}.${event.method}` === 'randomness.StoreRandomnessForEpoch') {
                            // Show what we are busy with
                            console.log(`EVENT=${event.section}:${event.method}; data=${event.data.toString()}`);
                            const [epochIndex, randomness] = event.data;
                            const randomnessFromEvent = randomness.toString();

                            const randomnessFromStorage = (
                                (await api.query.randomness.randomnessByEpochIndex(epochIndex)) as U64
                            ).toString();

                            // for epoch 1 randomness is always 0
                            if ((epochIndex as U64).toNumber() > 1) {
                                expect(randomnessFromEvent).not.toBe(
                                    '0x0000000000000000000000000000000000000000000000000000000000000000',
                                );
                            } else {
                                expect(randomnessFromEvent).toBe(
                                    '0x0000000000000000000000000000000000000000000000000000000000000000',
                                );
                            }

                            expect(randomnessFromEvent).toBe(randomnessFromStorage);

                            recordedEvents++;
                        }
                    } // loop over events

                    const currentBlock = (await getChainStatus(api)).bestNumber;
                    if (currentBlock - initialBlock >= maxBlocks) {
                        resolve(undefined);
                    }
                })
                .catch((error) => reject(error));
        }).then(() => {
            expect(recordedEvents).toBeGreaterThan(4);
        });
    }, 500_000); // 70 blocks is 350 sec + reserve to avoid timeouts
});
