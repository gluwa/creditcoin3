import { newApi, ApiPromise, KeyringPair } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import { chain_Anvil2_Key } from '../blockchain-tests/pallets/supported-chains/consts';

describe('handleAttestorElectionPolicyChanged()', () => {
    let api: ApiPromise;
    let root: KeyringPair;
    let startingBlock: bigint;

    const newPolicy = 'DeniedToAll';

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
    }, 30_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when election policy is updated', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0n);

            await api.tx.sudo
                .sudo(api.tx.attestation.setElectionPolicy(chain_Anvil2_Key, newPolicy))
                .signAndSend(root, { nonce: await api.rpc.system.accountNextIndex(root.address) });
            await forElapsedBlocks(api, { minBlocks: 2 });
        }, 30_000);

        it('graphQL returns ChangedElectionPolicy entity', async () => {
            const response = await graphQLQuery(
                `query { changedElectionPolicies(orderBy: BLOCK_NUMBER_ASC, last: 1, filter: {chainKey: {equalTo: "${chain_Anvil2_Key}"}}) { nodes { id, blockNumber, date, chainKey, electionPolicy }}}`,
            );
            expect(response.data.changedElectionPolicies.nodes).toBeTruthy();
            expect(response.data.changedElectionPolicies.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.changedElectionPolicies.nodes) {
                expect(node.id).toBeTruthy();
                // note: inspecting only last record
                expect(BigInt(node.blockNumber)).toBeGreaterThan(startingBlock);
                expect(Date.parse(node.date)).toBeGreaterThan(0);
                expect(Date.parse(node.date)).toBeLessThan(Date.now());
                expect(Number(node.chainKey)).toEqual(chain_Anvil2_Key);
                expect(node.electionPolicy).toEqual(newPolicy);
            }
        });

        it('graphQL returns updated AttestationChainData entity', async () => {
            const response = await graphQLQuery(
                `query { attestationChainData(orderBy: CHAIN_KEY_ASC, last: 1, filter: {chainKey: {equalTo: "${chain_Anvil2_Key}"}}) { nodes { id, electionPolicy }}}`,
            );
            expect(response.data.attestationChainData.nodes.length).toEqual(1);
            for (const node of response.data.attestationChainData.nodes) {
                expect(node.electionPolicy).toEqual(newPolicy);
            }
        });
    });
});
