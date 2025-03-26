import { WebSocketProvider, ethers } from 'ethers';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';

describe('handleEventProverDeployed()', () => {
    let alith: any;

    beforeAll(() => {
        const provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        // NOTE: chain starts with prover for Anvil 1 already running
    }, 30_000);

    describe('when there are provers running', () => {
        it('graphQL returns known Prover entity', async () => {
            const response = await graphQLQuery(
                `query { provers(orderBy: ID_ASC, last: 10) { nodes { id, owner, proceedsAccount, contractAddress, baseCostPerByte, baseFee, chainKey, name }}}`,
            );
            expect(response.data.provers.nodes).toBeTruthy();
            expect(response.data.provers.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.provers.nodes) {
                // these are EVM style addresses
                expect(node.owner).toEqual(alith.address);
                expect(node.proceedsAccount).toEqual(alith.address);
                expect(node.contractAddress.startsWith('0x')).toBeTruthy();
                expect(BigInt(node.baseCostPerByte)).toBeGreaterThan(0);
                expect(BigInt(node.baseFee)).toBeGreaterThan(0);
                // we only have provers for Anvil 1
                expect(parseInt(node.chainKey, 10)).toEqual(chain_Anvil1_Key);
                // this is hard-coded in ci.yaml
                expect(node.name).toEqual('Prover-1-for-Alice');

                // query each node individually to cover this endpoint too
                const response2 = await graphQLQuery(
                    `query { prover(id: "${node.id}") { id, owner, proceedsAccount, contractAddress, baseCostPerByte, baseFee, chainKey, name }}`,
                );
                expect(response2.data.prover).toBeTruthy();
                expect(response2.data.prover.id).toEqual(node.id);
                expect(response2.data.prover.owner).toEqual(node.owner);
                expect(response2.data.prover.proceedsAccount).toEqual(node.proceedsAccount);
                expect(response2.data.prover.contractAddress).toEqual(node.contractAddress);
                expect(response2.data.prover.baseCostPerByte).toEqual(node.baseCostPerByte);
                expect(response2.data.prover.baseFee).toEqual(node.baseFee);
                expect(response2.data.prover.chainKey).toEqual(node.chainKey);
                expect(response2.data.prover.name).toEqual(node.name);
            }
        });
    });
});
