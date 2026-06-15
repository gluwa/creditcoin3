import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { randomFundedAccount } from '../integration-tests/helpers';
import { graphQLQuery } from './common';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';

describe('handleAuthorizedAttestorRemoved()', () => {
    let api: ApiPromise;
    let bob: KeyringPair;
    let root: KeyringPair;
    let attestor: any;
    let startingBlock: bigint;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        bob = (global as any).CREDITCOIN_CREATE_SIGNER('bob');
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');

        attestor = await randomFundedAccount(api, root);

        // NOTE: Bob is the STASH for a random attestor on the Anvil2 chain
        await api.tx.attestation
            .registerAttestor(chain_Anvil2_Key, attestor.address)
            .signAndSend(bob, { nonce: await api.rpc.system.accountNextIndex(bob.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });

        await api.tx.sudo
            .sudo(api.tx.attestation.authorizeAttestor(chain_Anvil2_Key, attestor.address))
            .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when attestor is removed from authorized set', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.attestation.removeAuthorizedAttestor(chain_Anvil2_Key, attestor.address))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns AuthorizedAttestorRemoved entity', async () => {
            const response = await graphQLQuery(
                `query { authorizedAttestorRemoveds(orderBy: BLOCK_NUMBER_ASC, last: 1, filter: {attestorId: {equalTo: "${attestor.address}"}}) { nodes { id, blockNumber, date, chainKey }}}`,
            );
            expect(response.data.authorizedAttestorRemoveds.nodes).toBeTruthy();
            expect(response.data.authorizedAttestorRemoveds.nodes.length).toEqual(1);

            for (const node of response.data.authorizedAttestorRemoveds.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(Number(node.chainKey)).toEqual(chain_Anvil2_Key);
            }
        });

        it('graphQL returns no AuthorizedAttestors entity', async () => {
            const response = await graphQLQuery(
                `query { authorizedAttestors(orderBy: CHAIN_KEY_ASC, last: 100, filter: {attestorId: {equalTo: "${attestor.address}"}}) { nodes { id }}}`,
            );
            expect(response.data.authorizedAttestors.nodes.length).toEqual(0);
        });
    });
});
