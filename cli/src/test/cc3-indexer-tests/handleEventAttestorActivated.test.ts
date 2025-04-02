import { WasmPrivateKey } from 'bls-signatures-bindings';

import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount } from '../integration-tests/helpers';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventAttestorActivated()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let attestor: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        attestor = await randomFundedAccount(api, root);

        // NOTE: Bob is the STASH for a random attestor on the Anvil2 chain
        await api.tx.attestation.registerAttestor(chain_Anvil2_Key, attestor.address).signAndSend(bob);

        // wait for txn to make it on chain so we can deregister later
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 45_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when new attestor is activated', () => {
        let startingBlock = 0;

        beforeAll(async () => {
            // make sure attestor is reported as registered
            let response = await graphQLQuery(
                `query { attestors(orderBy: LAST_UPDATE_BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, stashId, chainKey, lastUpdateBlockNumber, status, blsPublicKey } } }`,
            );
            let foundMatch = false;
            for (const node of response.data.attestors.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                    expect(node.stashId).toEqual(bob.address);
                    expect(node.status).toEqual(1); // has just been registered
                }
            }
            expect(foundMatch).toEqual(true);

            // make sure this attestor is not reported as previously activated
            response = await graphQLQuery(
                `query { attestorActivateds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, attestorId, chainKey, blockNumber }}}`,
            );
            foundMatch = false;
            for (const node of response.data.attestorActivateds.nodes) {
                if (node.attestorId === attestor.address) {
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(false);

            const blsSecretKey = WasmPrivateKey.generate(attestor.secret);
            const blsPublicKey = blsSecretKey.public_key().as_bytes();
            const proofOfPossession = blsSecretKey.sign(blsPublicKey);

            startingBlock = (await getChainStatus(api)).bestNumber;
            expect(startingBlock).toBeGreaterThan(0);

            // NOTE: now start attesting and observe GraphQL responses below
            await api.tx.attestation
                .attest(chain_Anvil2_Key, blsPublicKey, proofOfPossession.as_bytes())
                .signAndSend(attestor.keyring);

            // wait for txn to make it on chain & indexer to ingest the block
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 45_000);

        it('graphQL returns known AttestorActivated entity', async () => {
            const response = await graphQLQuery(
                `query { attestorActivateds(orderBy: BLOCK_NUMBER_ASC, last: 1) { nodes { id, whoId, blockNumber, attestorId, chainKey, date, blsPublicKey }}}`,
            );
            expect(response.data.attestorActivateds.nodes).toBeTruthy();
            expect(response.data.attestorActivateds.nodes.length).toBeGreaterThanOrEqual(1);

            // note: inspecting only last entity
            for (const node of response.data.attestorActivateds.nodes) {
                expect(node.id).toBeTruthy();
                expect(node.whoId).toEqual(attestor.address);
                expect(node.blockNumber).toBeGreaterThan(startingBlock);
                expect(node.attestorId).toEqual(attestor.address);
                expect(node.chainKey).toEqual(chain_Anvil2_Key);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(node.blsPublicKey.startsWith('0x')).toEqual(true);
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
                        }
                    ) { nodes { id, attestorId, lastUpdateBlockNumber, status, blsPublicKey }}
                }`,
            );
            expect(response.data.attestors.nodes).toBeTruthy();
            expect(response.data.attestors.nodes.length).toEqual(1);

            for (const node of response.data.attestors.nodes) {
                expect(node.attestorId).toEqual(attestor.address);
                expect(node.lastUpdateBlockNumber).toBeGreaterThan(startingBlock);
                expect(node.status).toEqual(4);
                expect(node.blsPublicKey.startsWith('0x')).toEqual(true);
            }
        });
    });
});
