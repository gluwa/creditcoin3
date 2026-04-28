import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount, waitEras } from '../integration-tests/helpers';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventAttestorChilled()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let root: KeyringPair;
    let attestor: any;
    let startingBlock = 0n;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);

        // NOTE: Bob is the STASH for a random attestor on the Anvil1 chain
        await api.tx.attestation
            .registerAttestor(chain_Anvil1_Key, attestor.address)
            .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });

        const blsSecretKey = WasmPrivateKey.generate(new TextEncoder().encode(attestor.secret));
        const blsPublicKey = blsSecretKey.public_key().as_bytes();
        const proofOfPossession = blsSecretKey.sign(blsPublicKey);
        await api.tx.attestation
            .attest(chain_Anvil1_Key, blsPublicKey, proofOfPossession.as_bytes())
            .signAndSend(attestor.keyring, { nonce: await api.rpc.system.accountNextIndex(attestor.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });

        const epoch = (await api.query.babe.epochIndex()).toNumber();
        await api.tx.sudo
            .sudo(api.tx.attestation.forceElection(epoch + 1))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 120_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when attestor is chilled', () => {
        beforeAll(async () => {
            // make sure attestor is reported as active before it schedules chill
            const response = await graphQLQuery(
                `query {
                    attestors(
                        orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC,
                        last: 10,
                        filter: {
                            attestorId: { equalTo: "${attestor.address}"},
                            chainKey: { equalTo: "${chain_Anvil1_Key}"},
                        }
                    ) {
                        nodes { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey }
                    },

                    attestorChilleds(orderBy: BLOCK_NUMBER_ASC, last: 10) {
                        nodes { id, attestorId, chainKey, blockNumber }
                    },
                }`,
            );
            let foundMatch = false;
            for (const node of response.data.attestors.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.stashId).toEqual(bob.address);
                    expect(node.status).toEqual(0); // Active
                }
            }
            expect(foundMatch).toEqual(true);

            // make sure this attestor is not reported as previously Chilled
            foundMatch = false;
            for (const node of response.data.attestorChilleds.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(false);

            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            // NOTE: chill schedules Leaving for an active attestor; Idle is applied at epoch rotation.
            await api.tx.attestation
                .chill(chain_Anvil1_Key, attestor.address)
                .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });

            // wait for txn to make it on chain & indexer to ingest the successful call
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns Leaving Attestor entity before epoch rotation', async () => {
            const response = await graphQLQuery(
                `query {
                    attestors(
                        orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC,
                        last: 1,
                        filter: {
                            attestorId: { equalTo: "${attestor.address}"},
                            chainKey: { equalTo: "${chain_Anvil1_Key}"},
                        }
                    ) { nodes { id, attestorId, lastUpdateBlockNumber, status }}
                }`,
            );
            expect(response.data.attestors.nodes).toBeTruthy();
            expect(response.data.attestors.nodes.length).toEqual(1);

            for (const node of response.data.attestors.nodes) {
                expect(node.attestorId).toEqual(attestor.address);
                expect(BigInt(node.lastUpdateBlockNumber)).toBeGreaterThanOrEqual(startingBlock);
                expect(node.status).toEqual(3); // Leaving
            }
        });

        describe('after epoch rotation', () => {
            beforeAll(async () => {
                await waitEras(1, api);
                await forElapsedBlocks(api, { minBlocks: 3 });
            }, 180_000);

            it('graphQL returns known AttestorChilled entity', async () => {
                const response = await graphQLQuery(
                    `query {
                        attestorChilleds(
                            orderBy: BLOCK_NUMBER_ASC,
                            last: 1,
                            filter: {
                                attestorId: { equalTo: "${attestor.address}"},
                                chainKey: { equalTo: "${chain_Anvil1_Key}"},
                            }
                        ) { nodes { id, whoId, blockNumber, attestorId, chainKey, date }}
                    }`,
                );
                expect(response.data.attestorChilleds.nodes).toBeTruthy();
                expect(response.data.attestorChilleds.nodes.length).toBeGreaterThanOrEqual(1);

                // note: inspecting only last entity
                for (const node of response.data.attestorChilleds.nodes) {
                    expect(node.id).toBeTruthy();
                    // Epoch-triggered AttestorChilled has no extrinsic signer, so the indexer
                    // attributes the event to the attestor being chilled.
                    expect(node.whoId).toEqual(attestor.address);
                    expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                    expect(node.attestorId).toEqual(attestor.address);
                    expect(node.chainKey).toEqual(chain_Anvil1_Key.toString());
                    expect(Date.parse(node.date)).toBeGreaterThan(0);
                    expect(Date.parse(node.date)).toBeLessThan(Date.now());
                }
            });

            it('graphQL returns updated Attestor entity', async () => {
                const response = await graphQLQuery(
                    `query {
                        attestors(
                            orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC,
                            last: 1,
                            filter: {
                                attestorId: { equalTo: "${attestor.address}"},
                                chainKey: { equalTo: "${chain_Anvil1_Key}"},
                            }
                        ) { nodes { id, attestorId, lastUpdateBlockNumber, status }}
                    }`,
                );
                expect(response.data.attestors.nodes).toBeTruthy();
                expect(response.data.attestors.nodes.length).toEqual(1);

                for (const node of response.data.attestors.nodes) {
                    expect(node.attestorId).toEqual(attestor.address);
                    expect(BigInt(node.lastUpdateBlockNumber)).toBeGreaterThan(startingBlock);
                    expect(node.status).toEqual(1); // Idle/Chilled
                }
            });
        });
    });
});
