import { JsonRpcProvider, WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import solidityJSON = require('../blockchain-tests/artifacts/from-hardhat/ProverForTesting.sol/ProverForTesting.json');

describe('handleEscrowedPaymentReclaimed()', () => {
    let api: ApiPromise;
    let alith: any;
    let contract: any;
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

        // deploy a fake contract so we can have more control over query state(s)
        const factory = ethers.ContractFactory.fromSolidity(solidityJSON).connect(alith);
        contract = await factory.deploy(
            alith.address,
            10n,
            1000n,
            sampleQuery.chainId,
            'Prover-for-Testing',
            10, // timeout in seconds
        );
        await contract.waitForDeployment();
        expect((await contract.getAddress()).startsWith('0x')).toEqual(true);
    }, 30_000);

    describe('when an escrow payment is reclaimed', () => {
        let queryCost = 0n;
        let queryId = '';
        let queryDetailsOnChain;
        let startingBlock: number;

        beforeAll(async () => {
            // add variability so we don't trigger 'Query already exists' error from Prover.sol
            const provider_Anvil1 = new JsonRpcProvider((global as any).ANVIL1_URL);
            sampleQuery.height = await provider_Anvil1.getBlockNumber();

            queryCost = await contract.computeQueryCost(sampleQuery);
            expect(queryCost).toBeGreaterThan(0);

            // submit query as InvalidQuery so we can reclaim immediately
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
            expect(queryId).toBeTruthy();
            expect(queryId.startsWith('0x')).toEqual(true);

            queryDetailsOnChain = await contract.queries(queryId);
            expect(queryDetailsOnChain.state).toEqual(3n); // from the call above
            expect(queryDetailsOnChain.escrowedAmount).toEqual(queryCost);

            await forElapsedBlocks(api, { minBlocks: 1 });
            startingBlock = (await getChainStatus(api)).bestNumber;

            // reclaim so we can observe results
            await (await contract.reclaimEscrowedPayment(queryId)).wait();
            // check results on-chain
            queryDetailsOnChain = await contract.queries(queryId);
            expect(queryDetailsOnChain.state).toEqual(3n);
            expect(queryDetailsOnChain.escrowedAmount).toEqual(0n);

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known EscrowPaymentReclaimed entity', async () => {
            const response = await graphQLQuery(
                `query { escrowPaymentReclaimeds(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, blockNumber, who, amount }}}`,
            );
            expect(response.data.escrowPaymentReclaimeds.nodes).toBeTruthy();
            expect(response.data.escrowPaymentReclaimeds.nodes.length).toBeGreaterThan(0);

            let foundMatch = false;
            const contractAddress = (await contract.getAddress()).toLowerCase();
            for (const node of response.data.escrowPaymentReclaimeds.nodes) {
                expect(node.id).toBeTruthy();

                if (node.who === contractAddress && node.blockNumber > startingBlock) {
                    expect(BigInt(node.amount)).toEqual(queryCost);
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
