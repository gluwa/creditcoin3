import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';

const DEV_CHAIN = 2;

describe('BlockAttested events', (): void => {
    let api: ApiPromise;
    const maxBlocks = 220; // ~ 18:20 mins

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        // check that we have enough attestors
        const numAttestors = (await api.query.attestation.activeAttestors(DEV_CHAIN)).encodedLength;

        expect(numAttestors).toBeGreaterThanOrEqual(3);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    test('are emitted frequently enough and match Ethereum', async (): Promise<void> => {
        /* eslint-disable @typescript-eslint/naming-convention */
        const attestedEvents: { [key: string]: number } = {
            '2': 0,
            '4': 0,
        };
        const electionEvents: { [key: string]: number } = {
            '2': 0,
            '4': 0,
        };
        const initialBlock = (await getChainStatus(api)).bestNumber;

        return new Promise((resolve, reject): void => {
            // Subscribe to system events via storage
            api.query.system
                .events(async (events) => {
                    console.log(`Received ${events.length} events`);

                    // Loop through the Vec<EventRecord>
                    for (const record of events) {
                        const { event, phase: _ } = record;

                        if (`${event.section}.${event.method}` === 'attestation.BlockAttested') {
                            // Show what we are busy with
                            console.log(`EVENT=${event.section}:${event.method}; data=${event.data.toString()}`);
                            const [supportedChainKey] = event.data;
                            const supportedChainKeyStr = (supportedChainKey as U64).toString();

                            attestedEvents[supportedChainKeyStr]++;
                        }

                        if (`${event.section}.${event.method}` === 'attestation.AttestorsElected') {
                            // Show what we are busy with
                            console.log(`EVENT=${event.section}:${event.method}; data=${event.data.toString()}`);
                            const [_epoch, supportedChainKey, _attestors] = event.data;
                            const supportedChainKeyStr = (supportedChainKey as U64).toString();

                            electionEvents[supportedChainKeyStr]++;
                        }
                    } // loop over events

                    const currentBlock = (await getChainStatus(api)).bestNumber;
                    if (currentBlock - initialBlock >= maxBlocks) {
                        resolve(undefined);
                    }
                })
                .catch((error) => reject(error));
        }).then(async () => {
            // b/c we always start from scratch in CI expect that there is
            // a checkpoint for the genesis block of the ingested chain
            let checkpointsForGenesis = 0;
            const checkpoints = await api.query.attestation.checkpoints.entries(DEV_CHAIN);
            checkpoints.forEach(([_key, attestation]) => {
                if (attestation.unwrap().blockNumber.toNumber() === 0) {
                    checkpointsForGenesis++;
                }
            });
            expect(checkpointsForGenesis).toBe(1);

            expect(electionEvents['2']).toBeGreaterThanOrEqual(10);
            expect(electionEvents['4']).toBeGreaterThanOrEqual(10);

            expect(attestedEvents['2']).toBeGreaterThan(0);
            expect(attestedEvents['4']).toBeGreaterThan(0);

            // 200 CC blocks is 1000 seconds which means around 166 Anvil blocks
            // ingested at 10 blocks this means 15-16 events max
            expect(attestedEvents['2']).toBeLessThanOrEqual(20);
            expect(attestedEvents['4']).toBeLessThanOrEqual(20);

            // match the frequency b/c we don't want this to pass if only a few events are recorded
            // and then something suddenly fails/disconnects
            expect(attestedEvents['2']).toBeGreaterThanOrEqual(6);
            expect(attestedEvents['4']).toBeGreaterThanOrEqual(6);
            // note that this isn't super robust b/c we still don't quite know what the
            // average distance between these events is, see CSUB-1268 but
            // nevertheless should be good enough for CI to detect if something suddenly
            // starts failing
        });
    }, 1_500_000); // 220 blocks is 1100 sec + reserve to avoid timeouts
});
