import { JsonRpcProvider, WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import solidityJSON = require('../blockchain-tests/artifacts/from-hardhat/ProverForTesting.sol/ProverForTesting.json');

describe('handleQueryProofVerified()', () => {
    let api: ApiPromise;
    let alith: any;
    let contract: any;
    let queryId = '';
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
            'Prover-for-handleQueryProofVerified',
            1000, // timeout in seconds
        );
        await contract.waitForDeployment();
        expect((await contract.getAddress()).startsWith('0x')).toEqual(true);

        // add variability so we don't trigger 'Query already exists' error from Prover.sol
        const provider_Anvil1 = new JsonRpcProvider((global as any).ANVIL1_URL);
        sampleQuery.height = await provider_Anvil1.getBlockNumber();

        const queryCost = await contract.computeQueryCost(sampleQuery);

        // submit query as InvalidQuery so it doesn't get picked up by Prover
        const receipt = await (
            await contract.mock_submitQueryWithState(
                sampleQuery,
                alith.address,
                3, // QueryState.InvalidQuery
                { value: queryCost },
            )
        ).wait();
        // @ts-ignore
        queryId = receipt?.logs[0]?.args?.[0];
        expect(queryId.startsWith('0x')).toEqual(true);

        const response = await graphQLQuery(
            `query { proofs(orderBy: ID_ASC, last: 10) { nodes { id, queryRef, resultSegments }}}`,
        );
        let foundMatch = false;
        for (const node of response.data.proofs.nodes) {
            if (node.queryRef === queryId) {
                foundMatch = true;
            }
        }
        // no proof for this queryId recorded initially
        expect(foundMatch).toEqual(false);

        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 60_000);

    describe('when a new proof is submitted', () => {
        beforeAll(async () => {
            // verifier precompile result already defaults to 0 in the contract
            // await (await contract.mock_setVerifierResult(0)).wait();

            // mock resultSegments just so we have something to look for in the GraphQL output later
            await (await contract.mock_pushQueryResultSegment({ offset: 444n, abiBytes: new Uint8Array(32) })).wait();

            // simulate proof submission and observe results
            const proof = new Uint8Array(32);
            await (await contract.submitQueryProof(queryId, proof)).wait();

            // make sure submitQueryProof() worked
            const queryDetails = await contract.queries(queryId);
            // QueryState.ResultAvailable, was submitted as 3 initially
            expect(queryDetails.state).toEqual(2n);
            // submitQueryProof() will zero this before emitting event
            expect(queryDetails.escrowedAmount).toEqual(0n);

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 45_000);

        it('graphQL returns known Proof entity', async () => {
            const response = await graphQLQuery(
                `query { proofs(orderBy: ID_ASC, last: 10) { nodes { id, queryRef, resultSegments }}}`,
            );
            expect(response.data.proofs.nodes).toBeTruthy();
            expect(response.data.proofs.nodes.length).toBeGreaterThan(0);

            let foundMatch = false;
            for (const node of response.data.proofs.nodes) {
                expect(node.id).toBeTruthy();

                if (node.queryRef === queryId) {
                    expect(node.resultSegments.length).toEqual(1);
                    expect(BigInt(node.resultSegments[0].offset)).toEqual(444n);
                    expect(node.resultSegments[0].bytes).toEqual(
                        '0x0000000000000000000000000000000000000000000000000000000000000000',
                    );
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(true);
        });

        it('graphQL returns updated ChainQueries entity', async () => {
            const response = await graphQLQuery(
                `query { chainQueries(orderBy: ID_ASC, last: 10) { nodes { id, chainQueryId, chainKey, height, index, state }}}`,
            );
            expect(response.data.chainQueries.nodes).toBeTruthy();
            expect(response.data.chainQueries.nodes.length).toBeGreaterThan(0);

            let foundMatch = false;
            for (const node of response.data.chainQueries.nodes) {
                expect(node.id).toBeTruthy();
                expect(node.chainQueryId).toBeTruthy();
                expect(node.chainQueryId.startsWith('0x')).toEqual(true);
                expect(node.chainKey).toEqual(chain_Anvil1_Key);

                if (node.chainQueryId === queryId) {
                    expect(BigInt(node.height)).toEqual(BigInt(sampleQuery.height));
                    expect(BigInt(node.index)).toEqual(BigInt(sampleQuery.index));

                    // starts with state === 'Submitted'
                    expect(node.state).toEqual('ResultAvailable');
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
