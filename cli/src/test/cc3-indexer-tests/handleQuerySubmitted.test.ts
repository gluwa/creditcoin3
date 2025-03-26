import { JsonRpcProvider, WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import contractABIJSON = require('../blockchain-tests/artifacts/prover.json');
const contractABI = contractABIJSON.abi;

describe('handleQuerySubmitted()', () => {
    let api: ApiPromise;
    let alith: any;
    let initialCount = 0;
    let contractAddress = '';
    let provider: WebSocketProvider;
    const sampleQuery = {
        chainId: chain_Anvil1_Key,
        height: 33,
        index: 3,
        layoutSegments: [
            { offset: 0, size: 32 },
            { offset: 32, size: 64 },
        ],
    };

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        // NOTE: chain starts with prover for Anvil 1 already running
        let response = await graphQLQuery(
            `query { provers(orderBy: ID_ASC, last: 10) { nodes { id, owner, chainKey, contractAddress }}}`,
        );
        for (const node of response.data.provers.nodes) {
            if (node.owner === alith.address && parseInt(node.chainKey, 10) === chain_Anvil1_Key) {
                contractAddress = node.contractAddress;
            }
        }
        expect(contractAddress.startsWith('0x')).toEqual(true);

        response = await graphQLQuery(
            `query { chainQueries(orderBy: ID_ASC, last: 10) { nodes { id, chainQueryId, chainKey, height, index, layoutSegments, state, estimatedCost, escrowedAmount }}}`,
        );
        initialCount = response.data.chainQueries.nodes.length;
    }, 30_000);

    describe('when a new query is submitted', () => {
        let queryCost = 0n;
        let queryId = '';

        beforeAll(async () => {
            const provider_Anvil1 = new JsonRpcProvider((global as any).ANVIL1_URL);
            // add variability so we don't trigger 'Query already exists' error from Prover.sol
            sampleQuery.height = await provider_Anvil1.getBlockNumber();

            const contract = new ethers.Contract(contractAddress, contractABI, provider);
            queryCost = await contract.computeQueryCost(sampleQuery);
            expect(queryCost).toBeGreaterThan(0);

            const receipt = await (
                await (contract.connect(alith) as any).submitQuery(sampleQuery, alith.address, { value: queryCost })
            ).wait();
            // @ts-ignore
            queryId = receipt?.logs[0]?.args?.[0];
            expect(queryId).toBeTruthy();
            expect(queryId.startsWith('0x')).toEqual(true);

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 30_000);

        it('graphQL returns known ChainQueries entity', async () => {
            const response = await graphQLQuery(
                `query { chainQueries(orderBy: ID_ASC, last: 10) { nodes { id, chainQueryId, chainKey, height, index, layoutSegments, state, estimatedCost, escrowedAmount }}}`,
            );
            expect(response.data.chainQueries.nodes).toBeTruthy();
            expect(response.data.chainQueries.nodes.length).toBeGreaterThan(initialCount);

            let foundMatch = false;
            for (const node of response.data.chainQueries.nodes) {
                expect(node.id).toBeTruthy();
                expect(node.chainQueryId).toBeTruthy();
                expect(node.chainQueryId.startsWith('0x')).toEqual(true);
                expect(node.chainKey).toEqual(chain_Anvil1_Key);

                if (node.chainQueryId === queryId) {
                    expect(BigInt(node.height)).toEqual(BigInt(sampleQuery.height));
                    expect(BigInt(node.index)).toEqual(BigInt(sampleQuery.index));

                    // check incoming segments for validity
                    for (const segment of node.layoutSegments) {
                        expect(BigInt(segment.offset)).toBeGreaterThanOrEqual(0);
                        expect(BigInt(segment.size)).toBeGreaterThanOrEqual(0);
                    }

                    // compare against what was submitted
                    const expectedSegments = sampleQuery.layoutSegments.map((segment) => {
                        return {
                            // BigInt's are represented as strings in the GraphQL response
                            offset: segment.offset.toString(),
                            size: segment.size.toString(),
                        };
                    });
                    expect(node.layoutSegments).toEqual(expectedSegments);

                    expect(node.state).toEqual('Submitted');
                    expect(BigInt(node.estimatedCost)).toEqual(queryCost);
                    expect(BigInt(node.escrowedAmount)).toEqual(queryCost);

                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
