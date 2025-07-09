import { JsonRpcProvider, WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import solidityJSON = require('../blockchain-tests/artifacts/from-hardhat/ProverForTesting.sol/ProverForTesting.json');

describe('handleQueryProofVerificationFailed()', () => {
    let api: ApiPromise;
    let alith: any;
    let contract: any;
    let queryId = '';
    let queryCost = 0n;
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

        const provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        // deploy a fake contract so we can have more control
        const factory = ethers.ContractFactory.fromSolidity(solidityJSON).connect(alith);
        contract = await factory.deploy(
            alith.address,
            10n,
            1000n,
            sampleQuery.chainId,
            'Prover-for-handleQueryProofVerificationFailed',
            1000, // timeout in seconds
        );
        await contract.waitForDeployment();
        expect((await contract.getAddress()).startsWith('0x')).toEqual(true);

        // add variability so we don't trigger 'Query already exists' error from Prover.sol
        const provider_Anvil1 = new JsonRpcProvider((global as any).ANVIL1_URL);
        sampleQuery.height = await provider_Anvil1.getBlockNumber();
        queryCost = await contract.computeQueryCost(sampleQuery);

        // submit query as Submitted state so we can mark it as invalid
        const receipt = await (
            await contract.mock_submitQueryWithState(
                sampleQuery,
                alith.address,
                1, // QueryState.Submitted
                { value: queryCost },
            )
        ).wait();
        // @ts-ignore
        queryId = receipt?.logs[0]?.args?.[0];
        expect(queryId.startsWith('0x')).toEqual(true);

        await forElapsedBlocks(api, { minBlocks: 2 });
    }, 70_000);

    describe('when a query is marked as invalid', () => {
        let queryDetailsOnChain: any;
        const failureReason = 'Query layout mismatch';

        beforeAll(async () => {
            // Verify the query is initially in Submitted state
            queryDetailsOnChain = await contract.queries(queryId);
            expect(queryDetailsOnChain.state).toEqual(1n); // QueryState.Submitted
            expect(queryDetailsOnChain.escrowedAmount).toBeGreaterThan(0n);

            // Mark query as invalid - trigger QueryProofVerificationFailed event
            await (await contract.markAsInvalid(queryId, failureReason)).wait();

            // Verify the query state changed on-chain
            queryDetailsOnChain = await contract.queries(queryId);
            expect(queryDetailsOnChain.state).toEqual(3n); // QueryState.InvalidQuery
            expect(queryDetailsOnChain.escrowedAmount).toEqual(0n); // Refunded

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 45_000);

        it('ChainQueries entity is correctly updated with InvalidQuery state and failure reason', async () => {
            const response = await graphQLQuery(
                `query {
                    chainQueries(
                        orderBy: ID_ASC,
                        last: 1,
                        filter: { chainQueryId: { equalTo: "${queryId}" }},
                    ) {
                        nodes {
                            id,
                            chainQueryId,
                            chainKey,
                            height,
                            index,
                            state,
                            escrowedAmount,
                            estimatedCost,
                            failedReason,
                            proverId
                        }
                    }
                }`,
            );
            expect(response.data.chainQueries.nodes).toBeTruthy();
            expect(response.data.chainQueries.nodes.length).toEqual(1);

            const node = response.data.chainQueries.nodes[0];

            // Verify query fields, not sure if these will be kept tho
            expect(node.id).toBeTruthy();
            expect(node.chainQueryId).toEqual(queryId);
            expect(node.chainKey).toEqual(chain_Anvil1_Key.toString());
            expect(BigInt(node.height)).toEqual(BigInt(sampleQuery.height));
            expect(BigInt(node.index)).toEqual(BigInt(sampleQuery.index));

            // Verify state is updated to InvalidQuery
            expect(node.state).toEqual('InvalidQuery');

            // Verify escrowedAmount is refunded
            expect(BigInt(node.escrowedAmount)).toEqual(0n);

            // Verify estimatedCost is preserved
            expect(BigInt(node.estimatedCost)).toEqual(queryCost);

            // Verify failedReason is correctly set
            expect(node.failedReason).toEqual(failureReason);
        });
    });
});
