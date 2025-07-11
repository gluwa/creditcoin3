import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { chain_Anvil1_Key } from '../blockchain-tests/pallets/supported-chains/consts';
import { graphQLQuery } from './common';
import { forElapsedBlocks } from '../utils';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import solidityJSON = require('../blockchain-tests/artifacts/from-hardhat/ProverForTesting.sol/ProverForTesting.json');

describe('handleEventProverDeployed()', () => {
    let alith: any;
    let balthathar: any;
    let api: ApiPromise;
    let contract: any;
    let contractAddress = '';

    beforeAll(async () => {
        const provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);
        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey).connect(provider);

        // NOTE: chain starts with prover for Anvil 1 already running
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        // deploy a fake contract so we can assert there's a prover with its address
        balthathar = new ethers.Wallet((global as any).CREDITCOIN_EVM_PRIVATE_KEY('bob')).connect(provider);
        const factory = ethers.ContractFactory.fromSolidity(solidityJSON).connect(balthathar);
        contract = await factory.deploy(
            // proceeds go to Alith b/c of asserts down below
            alith.address,
            11n,
            1111n,
            chain_Anvil1_Key,
            'Prover-for-Balthathar',
            10, // timeout in seconds
        );
        await contract.waitForDeployment();
        // b/c cc3-indexer records this as lowercase but ethers.js returns it in upper case
        contractAddress = (await contract.getAddress()).toLowerCase();
        expect(contractAddress.startsWith('0x')).toEqual(true);

        // give indexer time to process this
        await forElapsedBlocks(api, { minBlocks: 3 });
    }, 60_000);

    describe('when there are provers running', () => {
        it('graphQL returns known Prover entities', async () => {
            const response = await graphQLQuery(
                `query { provers(
                    orderBy: ID_ASC,
                    last: 50,
                    filter: { chainKey: { equalTo: "${chain_Anvil1_Key}" }},
                ) { nodes { id, owner, proceedsAccount, contractAddress, baseCostPerByte, baseFee, chainKey, name }}}`,
            );
            expect(response.data.provers.nodes).toBeTruthy();
            // min 2: 1x for Alith + 1x for Balthathar
            expect(response.data.provers.nodes.length).toBeGreaterThanOrEqual(2);

            let proverForBalthathar = false;
            for (const node of response.data.provers.nodes) {
                // these are EVM style addresses
                expect([alith.address, balthathar.address]).toContain(node.owner);
                expect(node.proceedsAccount).toEqual(alith.address);
                expect(node.contractAddress.startsWith('0x')).toEqual(true);
                if (node.contractAddress === contractAddress) {
                    proverForBalthathar = true;
                }
                // default values or greater
                expect(BigInt(node.baseCostPerByte)).toBeGreaterThanOrEqual(10n);
                expect(BigInt(node.baseFee)).toBeGreaterThanOrEqual(1000n);
                // we only have provers for Anvil 1
                expect(parseInt(node.chainKey, 10)).toEqual(chain_Anvil1_Key);
                // name from ci.yaml & other tests
                expect(node.name.startsWith('Prover-for-')).toEqual(true);

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
            expect(proverForBalthathar).toEqual(true);
        });
    });
});
