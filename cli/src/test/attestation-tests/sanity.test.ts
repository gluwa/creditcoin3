import { U64 } from '@polkadot/types-codec';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil1_Key, chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';

function randomIntBetween(min: number, max: number) {
    return min + Math.floor(Math.random() * (max - min));
}

describe('BlockAttested events', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    const maxBlocks = 220; // ~ 18:20 mins

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // check that we have enough attestors
        const attestorsForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).encodedLength;
        expect(attestorsForAnvil1).toBeGreaterThanOrEqual(3);

        const attestorsForAnvil2 = (await api.query.attestation.activeAttestors(chain_Anvil2_Key)).encodedLength;
        expect(attestorsForAnvil2).toBeGreaterThanOrEqual(3);
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
        const intervalChangedEvents: { [key: string]: number } = {
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

                        if (`${event.section}.${event.method}` === 'attestation.AttestationIntervalChanged') {
                            // Show what we are busy with
                            console.log(`EVENT=${event.section}:${event.method}; data=${event.data.toString()}`);
                            const [chainKey, _interval] = event.data;
                            const chainKeyStr = (chainKey as U64).toString();

                            intervalChangedEvents[chainKeyStr]++;
                        }

                        if (`${event.section}.${event.method}` === 'attestation.AttestorsElected') {
                            // Show what we are busy with
                            console.log(`EVENT=${event.section}:${event.method}; data=${event.data.toString()}`);
                            const [epoch, chainKey, _attestors] = event.data;
                            const supportedChainKeyStr = (chainKey as U64).toString();

                            electionEvents[supportedChainKeyStr]++;

                            const chainKeyAsNum = (chainKey as U64).toNumber();
                            const epochAsNum = (epoch as U64).toNumber();
                            if (epochAsNum % 2 === 0 && chainKeyAsNum === chain_Anvil2_Key) {
                                const defaultInterval = (
                                    api.consts.attestation.defaultAttestationInterval as U64
                                ).toNumber();
                                const newInterval = randomIntBetween(defaultInterval - 5, defaultInterval + 5);

                                // note: using chain Anvil-2 b/c changing interval for Anvil-1
                                // may lead to side effects in other test scenarios
                                await api.tx.sudo
                                    .sudo(api.tx.attestation.setChainAttestationInterval(chain_Anvil2_Key, newInterval))
                                    .signAndSend(root);
                                console.log(`**** DEBUG: NEW INTERVAL for ${chain_Anvil2_Key} will be ${newInterval}`);
                            }
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
            const checkpoints = await api.query.attestation.checkpoints.entries(chain_Anvil1_Key);
            checkpoints.forEach(([_key, attestation]) => {
                if (attestation.unwrap().blockNumber.toNumber() === 0) {
                    checkpointsForGenesis++;
                }
            });
            expect(checkpointsForGenesis).toBe(1);

            expect(electionEvents[chain_Anvil1_Key]).toBeGreaterThanOrEqual(10);
            expect(electionEvents[chain_Anvil2_Key]).toBeGreaterThanOrEqual(10);

            expect(attestedEvents[chain_Anvil1_Key]).toBeGreaterThan(0);
            expect(attestedEvents[chain_Anvil2_Key]).toBeGreaterThan(0);

            // 200 CC blocks is 1000 seconds which means around 166 Anvil blocks
            // ingested at 10 blocks this means 15-16 events max
            expect(attestedEvents[chain_Anvil1_Key]).toBeLessThanOrEqual(25);
            expect(attestedEvents[chain_Anvil2_Key]).toBeLessThanOrEqual(25);

            // match the frequency b/c we don't want this to pass if only a few events are recorded
            // and then something suddenly fails/disconnects
            expect(attestedEvents[chain_Anvil1_Key]).toBeGreaterThanOrEqual(6);
            expect(attestedEvents[chain_Anvil2_Key]).toBeGreaterThanOrEqual(6);
            // note that this isn't super robust b/c we still don't quite know what the
            // average distance between these events is, see CSUB-1268 but
            // nevertheless should be good enough for CI to detect if something suddenly
            // starts failing

            expect(intervalChangedEvents[chain_Anvil1_Key]).toBe(0);
            // this test loops over roughly 15 epochs and we make a change every 2
            expect(intervalChangedEvents[chain_Anvil2_Key]).toBeGreaterThanOrEqual(5);
        });
    }, 1_500_000); // 220 blocks is 1100 sec + reserve to avoid timeouts
});
