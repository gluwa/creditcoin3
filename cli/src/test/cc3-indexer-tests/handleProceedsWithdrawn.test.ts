import { JsonRpcProvider, WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';
import solidityJSON = require('../blockchain-tests/artifacts/from-hardhat/ProverForTesting.sol/ProverForTesting.json');

describe('handleProceedsWithdrawn()', () => {
    let api: ApiPromise;
    let alith: any;
    let contract: any;
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
            'Prover-for-handleProceedsWithdrawn',
            10, // timeout in seconds
        );
        await contract.waitForDeployment();
        expect((await contract.getAddress()).startsWith('0x')).toEqual(true);

        // add variability so we don't trigger 'Query already exists' error from Prover.sol
        const provider_Anvil1 = new JsonRpcProvider((global as any).ANVIL1_URL);
        sampleQuery.height = await provider_Anvil1.getBlockNumber();

        const queryCost = await contract.computeQueryCost(sampleQuery);

        // submit query as InvalidQuery so it doesn't get picked up by Prover
        await (
            await contract.mock_submitQueryWithState(
                sampleQuery,
                alith.address,
                3, // QueryState.InvalidQuery
                { value: queryCost },
            )
        ).wait();

        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 60_000);

    describe('when proceeds are withdrawn', () => {
        let startingBlock: number;

        beforeAll(async () => {
            const escrowBefore = await contract.getTotalEscrowBalance();
            expect(escrowBefore).toBeGreaterThan(0);
            // manipulate totalEscrowBalance so the contrac thinks that
            // there is something left to be withdrawn (instead of being paid out immediately to prover)
            await (await contract.mock_drainTotalEscrowBalance(1111n)).wait();

            // make sure it worked
            const escrowAfter = await contract.getTotalEscrowBalance();
            expect(escrowAfter).toBeGreaterThan(0);
            expect(escrowAfter).toEqual(escrowBefore - 1111n);

            startingBlock = (await getChainStatus(api)).bestNumber;

            // withdraw and observe results
            await (await contract.withdrawProceeds()).wait();
            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 60_000);

        it('graphQL returns known ProceedsWithdrawn entity', async () => {
            const response = await graphQLQuery(
                `query { proceedsWithdrawns(orderBy: BLOCK_NUMBER_ASC, last: 10) { nodes { id, blockNumber, who, proceedsAccount, amount }}}`,
            );
            expect(response.data.proceedsWithdrawns.nodes).toBeTruthy();
            expect(response.data.proceedsWithdrawns.nodes.length).toBeGreaterThan(0);

            // WARNING: for some reason node.who is reported as lower-case while
            // node.proceedsAccount matches verbatim
            const contractAddress = (await contract.getAddress()).toLowerCase();
            let foundMatch = false;
            for (const node of response.data.proceedsWithdrawns.nodes) {
                expect(node.id).toBeTruthy();

                if (node.who === contractAddress && node.blockNumber >= startingBlock) {
                    expect(node.proceedsAccount).toEqual(alith.address);
                    expect(BigInt(node.amount)).toEqual(1111n);
                    foundMatch = true;
                }
            }
            expect(foundMatch).toEqual(true);
        });
    });
});
