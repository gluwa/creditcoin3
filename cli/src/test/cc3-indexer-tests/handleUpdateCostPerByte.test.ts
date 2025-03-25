import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import contractABIJSON = require('../blockchain-tests/artifacts/prover.json');
const contractABI = contractABIJSON.abi;

describe('handleUpdateCostPerByte()', () => {
    let api: ApiPromise;
    let alith: any;
    let initialCostPerByte = 0n;
    let contractAddress = '';
    let provider: WebSocketProvider;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        // NOTE: chain starts with prover for Anvil 1 already running
        const response = await graphQLQuery(
            `query { provers(
                orderBy: ID_ASC,
                last: 10,
                filter: { chainKey: { equalTo: "${chain_Anvil1_Key}" }},
            ) { nodes { id, owner, proceedsAccount, contractAddress, baseCostPerByte, baseFee, chainKey, name }}}`,
        );
        for (const node of response.data.provers.nodes) {
            if (node.owner === alith.address) {
                initialCostPerByte = BigInt(node.baseCostPerByte);
                contractAddress = node.contractAddress;
                // NOTE: will operate on contract for last prover deployed for this source chain
            }
        }
        expect(initialCostPerByte).toBeGreaterThan(0);
        expect(contractAddress.startsWith('0x')).toEqual(true);
    }, 30_000);

    describe('when prover updates their cost per byte', () => {
        let newCostPerByte: bigint;

        beforeAll(async () => {
            newCostPerByte = initialCostPerByte + 4n;

            const contract = new ethers.Contract(contractAddress, contractABI, provider);

            await (await (contract.connect(alith) as any).updateCostPerByte(newCostPerByte)).wait();
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 45_000);

        it('graphQL returns updated Prover entities', async () => {
            const response = await graphQLQuery(
                `query {
                    provers(
                        orderBy: ID_ASC,
                        filter: { contractAddress: { equalTo: "${contractAddress}" }},
                    ) { nodes { id, owner, proceedsAccount, contractAddress, baseCostPerByte, baseFee, chainKey, name }}}`,
            );
            expect(response.data.provers.nodes).toBeTruthy();
            expect(response.data.provers.nodes.length).toBeGreaterThanOrEqual(1);

            for (const node of response.data.provers.nodes) {
                // these are EVM style addresses
                expect(node.owner).toEqual(alith.address);
                expect(node.proceedsAccount).toEqual(alith.address);
                // we only have provers for Anvil 1
                expect(parseInt(node.chainKey, 10)).toEqual(chain_Anvil1_Key);
                // this is hard-coded in ci.yaml
                expect(node.name.startsWith('Prover-for-')).toEqual(true);

                // fee was updated
                expect(BigInt(node.baseCostPerByte)).toEqual(newCostPerByte);
            }
        });
    });
});
