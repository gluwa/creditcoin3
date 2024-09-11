import { U64 } from '@polkadot/types-codec';
import { AttestorPrimitivesSignedAttestation } from '@polkadot/types/lookup';
import { newApi, ApiPromise } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';

describe('BlockAttested events', (): void => {
    let api: ApiPromise;
    const maxBlocks = 200; // ~ 16:40 mins

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        // check that we have enough attestors
        const numAttestors = (await api.query.attestation.counterForAttestors()).toNumber();
        expect(numAttestors).toBeGreaterThanOrEqual(5);
    });

    afterAll(async () => {
        await api.disconnect();
    });

    test('are emitted frequently enough and match Ethereum', async (): Promise<void> => {
        let previousDigest = '';
        let previousHeader = 0;
        let attestedEvents = 0;
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
                            const [chainId, signedAttn, digest] = event.data;
                            const data = signedAttn as AttestorPrimitivesSignedAttestation;

                            const chainAttestationInterval = (
                                (await api.query.attestation.chainAttestationInterval(chainId)) as U64
                            ).toNumber();

                            // external blocks should be attested at the same interval which is recorded on-chain
                            if (previousHeader > 0) {
                                expect(data.attestation.headerNumber.toNumber() - previousHeader).toBe(
                                    chainAttestationInterval,
                                );
                            }
                            previousHeader = data.attestation.headerNumber.toNumber();

                            // recorded attestations should be linked to each other
                            if (previousDigest !== '') {
                                expect(data.attestation.prevDigest.toString()).toBe(previousDigest);
                            }
                            // note: next attestation will point to the digest of the current one
                            previousDigest = digest.toString();

                            attestedEvents++;
                        }
                    } // loop over events

                    const currentBlock = (await getChainStatus(api)).bestNumber;
                    if (currentBlock - initialBlock >= maxBlocks) {
                        resolve(undefined);
                    }
                })
                .catch((error) => reject(error));
        }).then(() => {
            expect(attestedEvents).toBeGreaterThan(0);

            // 200 CC blocks is 1000 seconds which means around 166 Anvil blocks
            // ingested at 10 blocks this means 15-16 events max
            expect(attestedEvents).toBeLessThanOrEqual(20);

            // match the frequency b/c we don't want this to pass if only a few events are recorded
            // and then something suddenly fails/disconnects
            expect(attestedEvents).toBeGreaterThanOrEqual(10);
            // note that this isn't super robust b/c we still don't quite know what the
            // average distance between these events is, see CSUB-1268 but
            // nevertheless should be good enough for CI to detect if something suddenly
            // starts failing
        });
    }, 1_500_000); // 200 blocks is 1000 sec + reserve to avoid timeouts
});
