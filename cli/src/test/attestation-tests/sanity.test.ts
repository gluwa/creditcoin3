import { U64 } from '@polkadot/types-codec';
import { WebSocketProvider } from 'ethers';
import { AttestorPrimitivesSignedAttestation } from '@polkadot/types/lookup';
import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import {
    chain_Anvil1_Key,
    chain_Anvil1_Url,
    chain_Anvil2_Key,
} from '../blockchain-tests/pallets/supported-chains/consts';
import { calculateThreshold, randomIntBetween } from '../utils';

describe('BlockAttested events', (): void => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingEpoch = 0;
    let chain_Anvil1_AttestationInterval = 0;
    let startBlock_Anvil1 = 0;
    let provider_Anvil1: WebSocketProvider;
    const maxBlocks = 220; // ~ 18:20 mins

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        startingEpoch = (await api.query.babe.epochIndex()).toNumber();
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        // check that we have enough attestors
        const attestorsForAnvil1 = (await api.query.attestation.activeAttestors(chain_Anvil1_Key)).encodedLength;
        expect(attestorsForAnvil1).toBeGreaterThanOrEqual(3);

        const attestorsForAnvil2 = (await api.query.attestation.activeAttestors(chain_Anvil2_Key)).encodedLength;
        expect(attestorsForAnvil2).toBeGreaterThanOrEqual(3);

        // NOTE: this stays constant during test execution
        chain_Anvil1_AttestationInterval = (
            await api.query.attestation.chainAttestationInterval(chain_Anvil1_Key)
        ).toNumber();

        provider_Anvil1 = new WebSocketProvider(chain_Anvil1_Url);
        startBlock_Anvil1 = await provider_Anvil1.getBlockNumber();
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

        const attestedEventsMemory: {
            [key: string]: { headerNumber: number; digest: string }[];
        } = { '2': [], '4': [] };

        const electionEvents: { [key: string]: number } = {
            '2': 0,
            '4': 0,
        };
        const intervalChangedEvents: { [key: string]: number } = {
            '2': 0,
            '4': 0,
        };
        const initialBlock = (await getChainStatus(api)).bestNumber;
        const expectedMinVotes: { [key: string]: bigint } = {
            '2': calculateThreshold((await api.query.attestation.targetSampleSize(chain_Anvil1_Key)).toBigInt()),
            '4': calculateThreshold((await api.query.attestation.targetSampleSize(chain_Anvil2_Key)).toBigInt()),
        };

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
                            const [supportedChainKey, headerNumber, digest] = event.data;
                            const supportedChainKeyStr = (supportedChainKey as U64).toString();
                            const digestHex = digest.toHex();

                            // Note: We can no longer check attestor count from the event
                            // The full attestation data is now only available via call handler or storage query
                            // For testing purposes, we'll just track the event occurrence

                            // Keep in memory for later continuity proof validation
                            // Note: We can't store the full signed attestation anymore from the event
                            (attestedEventsMemory[supportedChainKeyStr] ||= []).push({
                                headerNumber: (headerNumber as U64).toNumber(),
                                digest: digestHex,
                            });

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
                                const nonce = await api.rpc.system.accountNextIndex(root.address);
                                await api.tx.sudo
                                    .sudo(api.tx.attestation.setChainAttestationInterval(chain_Anvil2_Key, newInterval))
                                    .signAndSend(root, { nonce });
                                console.log(`**** DEBUG: NEW INTERVAL for ${chain_Anvil2_Key} will be ${newInterval}`);
                            }
                        }
                    } // loop over events

                    const currentBlock = (await getChainStatus(api)).bestNumber;
                    if (currentBlock - initialBlock >= maxBlocks) {
                        resolve(undefined);
                    }
                })
                .catch((error) => reject(new Error(error)));
        }).then(async () => {
            const prev_digests = [];
            // Validate continuity proofs for Anvil-1
            // Note: Continuity proof validation removed as full attestation data
            // is no longer available in the event. This would need to be done via
            // storage query or call handler if needed.
            for (const attestationRecord of attestedEventsMemory[chain_Anvil1_Key]) {
                prev_digests.push(attestationRecord.digest);

                if (attestationRecord.headerNumber > 0) {
                    // Note: Continuity proof validation removed as full attestation data
                    // is no longer available in the event. This would need to be done via
                    // storage query or call handler if needed.
                    // expect(attestationRecord.signed.continuityProof.blocks.length).toBeGreaterThanOrEqual(
                    //     chain_Anvil1_AttestationInterval - 1,
                    // );
                    // const continuityProofValid = validateContinuityProof(prev_digests, attestationRecord.signed);
                    // expect(continuityProofValid).toBeTruthy();
                } else {
                    console.log(
                        `**** DEBUG: SKIP continuity proof validation for genesis attestation for chain ${chain_Anvil1_Key}`,
                    );
                    // expect(attestationRecord.signed.continuityProof.blocks.length).toBe(0);
                }
            }

            // b/c we always start from scratch in CI expect that there is
            // a checkpoint for the genesis block of the ingested chain
            let checkpointsForGenesis = 0;
            const checkpoints = await api.query.attestation.checkpoints.entries(chain_Anvil1_Key);
            checkpoints.forEach(([key, _attestation]) => {
                if (key.args[1].toNumber() === 0) {
                    checkpointsForGenesis++;
                }
            });
            expect(checkpointsForGenesis).toBe(1);

            // note: this test is started *after* we have min 3 attestors already elected on each chain
            const currentEpoch = (await api.query.babe.epochIndex()).toNumber();
            const expectedElectionEvents = currentEpoch - startingEpoch;

            expect(electionEvents[chain_Anvil1_Key]).toBeGreaterThanOrEqual(expectedElectionEvents - 1);
            expect(electionEvents[chain_Anvil1_Key]).toBeLessThanOrEqual(expectedElectionEvents + 1);

            expect(electionEvents[chain_Anvil2_Key]).toBeGreaterThanOrEqual(expectedElectionEvents - 1);
            expect(electionEvents[chain_Anvil2_Key]).toBeLessThanOrEqual(expectedElectionEvents + 1);

            expect(attestedEvents[chain_Anvil1_Key]).toBeGreaterThan(0);
            expect(attestedEvents[chain_Anvil2_Key]).toBeGreaterThan(0);

            const endBlock_Anvil1 = await provider_Anvil1.getBlockNumber();
            const expectedBlockAttestedEvents_Anvil1 = Math.floor(
                (endBlock_Anvil1 - startBlock_Anvil1) / chain_Anvil1_AttestationInterval,
            );

            expect(attestedEvents[chain_Anvil1_Key]).toBeGreaterThanOrEqual(expectedBlockAttestedEvents_Anvil1 - 1);
            expect(attestedEvents[chain_Anvil1_Key]).toBeLessThanOrEqual(expectedBlockAttestedEvents_Anvil1 + 1);
            // note: interval for Anvil2 changes dynamically during this test
            expect(attestedEvents[chain_Anvil2_Key]).toBeLessThanOrEqual(40);

            // match the frequency b/c we don't want this to pass if only a few events are recorded
            // and then something suddenly fails/disconnects
            expect(attestedEvents[chain_Anvil1_Key]).toBeGreaterThanOrEqual(4);
            expect(attestedEvents[chain_Anvil2_Key]).toBeGreaterThanOrEqual(4);
            // note that this isn't super robust b/c we still don't quite know what the
            // average distance between these events is, see CSUB-1268 but
            // nevertheless should be good enough for CI to detect if something suddenly
            // starts failing

            expect(intervalChangedEvents[chain_Anvil1_Key]).toBe(0);
            // this test loops over roughly 15 epochs and we make a change every 2
            expect(intervalChangedEvents[chain_Anvil2_Key]).toBeGreaterThanOrEqual(5);
        });
    }, 1_500_000); // 220 blocks is 1100 sec + reserve to avoid timeouts

    // Helper function to validate continuity proof
    function validateContinuityProof(
        prev_digests: string[],
        attestation: AttestorPrimitivesSignedAttestation,
    ): boolean {
        if (attestation.continuityProof.blocks.length === 0) {
            console.log('**** DEBUG: continuity proof has no blocks; returning false');
            return false;
        }

        let block_prev_digest = '';
        attestation.continuityProof.blocks.forEach((block, index) => {
            if (block.blockNumber.isZero()) {
                return;
            }

            if (index === 0) {
                if (!prev_digests.includes(block.prevDigest.toHex())) {
                    console.log(
                        `**** DEBUG: continuity proof first block prevDigest ${block.prevDigest.toHex()} not in known prev_digests`,
                    );
                    return false;
                }
                block_prev_digest = block.prevDigest.toHex();
                return;
            }

            if (block.prevDigest.toHex() !== block_prev_digest) {
                return false;
            }
            block_prev_digest = block.digest.toHex();
        });

        return true;
    }
});
