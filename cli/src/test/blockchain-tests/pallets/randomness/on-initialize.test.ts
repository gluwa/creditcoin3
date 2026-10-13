import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise } from '../../../../lib';
import { getChainStatus } from '../../../../lib/chain/status';
import { describeIf } from '../../../utils';

describeIf(process.env.SKIP_ON_PURPOSE === undefined, 'StoreRandomnessForEpoch events', (): void => {
    let api: ApiPromise;
    // Hard ceiling only: the test stops once `minEvents` epoch boundaries have been checked,
    // which takes one to two epochs (15 blocks each on fast-runtime).
    const maxBlocks = 70; // > 5 min
    const minEvents = 2;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
    });

    afterAll(async () => {
        await api.disconnect();
    });

    test('are emitted frequently and randomness can be queried from storage', async (): Promise<void> => {
        let recordedEvents = 0;
        const initialBlock = (await getChainStatus(api)).bestNumber;
        let done = false;
        let unsubscribe: (() => void) | undefined;

        return new Promise((resolve, reject): void => {
            // Subscribe to system events via storage
            api.query.system
                .events(async (events) => {
                    if (done) {
                        return;
                    }
                    console.log(`Received ${events.length} events`);

                    // Loop through the Vec<EventRecord>
                    for (const record of events) {
                        const { event, phase: _ } = record;

                        if (`${event.section}.${event.method}` === 'randomness.StoreRandomnessForEpoch') {
                            // Show what we are busy with
                            console.log(`EVENT=${event.section}:${event.method}; data=${event.data.toString()}`);
                            const [epochIndex, randomness] = event.data;
                            const randomnessFromEvent = randomness.toString();

                            const epochIndexU64 = epochIndex as U64;
                            const epochNum = epochIndexU64.toNumber();

                            const randomnessMap = await api.query.randomness.randomnessByEpochIndex();
                            // We need to iterate instead of using .get() because the key is a U64 and we need to compare the value, not the reference
                            // since BTreeMap is a Map, we can use the entries() method to get an iterator of [key, value] pairs and find the one with the matching epoch index
                            const entry = [...randomnessMap.entries()].find(([k]) => k.toNumber() === epochNum);
                            const randomessRaw = entry?.[1];

                            expect(randomessRaw).toBeDefined();

                            const randomnessFromStorage = randomessRaw!.toString();

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
                    if (recordedEvents >= minEvents || currentBlock - initialBlock >= maxBlocks) {
                        done = true;
                        unsubscribe?.();
                        resolve(undefined);
                    }
                })
                .then((unsub) => {
                    unsubscribe = unsub;
                })
                .catch((error) => reject(new Error(error)));
        }).then(() => {
            expect(recordedEvents).toBeGreaterThanOrEqual(minEvents);
        });
    }, 500_000); // 70 blocks is 350 sec + reserve to avoid timeouts
});
