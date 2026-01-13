import * as proof from '@gluwa/cc-next-query-builder/dist/proof-generator';
import { EncodingVersion } from '@gluwa/cc-next-query-builder/dist/encodings';
import { WebSocketProvider, ethers } from 'ethers';
import { newApi, ApiPromise } from '../../lib';
import { getChainStatus } from '../../lib/chain/status';
import { chain_Anvil1_Key, chain_Anvil1_Url } from '../blockchain-tests/pallets/supported-chains/consts';
import { blockProverAddress } from '../blockchain-tests/precompiles/consts';
import { forElapsedBlocks } from '../utils';
import { graphQLQuery } from './common';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import blockProverABIJSON = require('../blockchain-tests/artifacts/block_prover.json');
const blockProverABI = blockProverABIJSON as unknown as ethers.InterfaceAbi;

describe('handleTransactionVerified()', () => {
    let blockProverContract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let startingBlock: bigint;
    let proofData: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);

        blockProverContract = new ethers.Contract(blockProverAddress, blockProverABI, alith);

        // we need a running Anvil at this address
        const anvil1Provider = new WebSocketProvider(chain_Anvil1_Url);

        // this value needs to be passed from the outside
        const transactionHash = process.env.ANVIL1_TXN_HASH;
        expect(transactionHash).toBeTruthy();

        // make sure we have attestations for this source block
        const sourceTxn = await anvil1Provider.getTransaction(transactionHash!);
        expect(sourceTxn).toBeDefined();
        expect(sourceTxn!.blockNumber).toBeDefined();

        const chainInfoProvider = new proof.chainInfo.PrecompileChainInfoProvider(provider);
        await chainInfoProvider.waitUntilHeightAttested(chain_Anvil1_Key, sourceTxn!.blockNumber!);
        // we're now sure that there are enough attestations on the execution chain

        const blockProvider = new proof.raw.blockProvider.SimpleBlockProvider(anvil1Provider);
        const rawProofGenerator = new proof.raw.RawProofGenerator(
            chain_Anvil1_Key,
            blockProvider,
            chainInfoProvider,
            EncodingVersion.V1,
        );
        const rawProofResult = await rawProofGenerator.generateProof(transactionHash!);
        expect(rawProofResult.success).toBe(true);
        proofData = rawProofResult.data!;
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    describe('when a new transaction is verified', () => {
        beforeAll(async () => {
            startingBlock = BigInt((await getChainStatus(api)).bestNumber);
            expect(startingBlock).toBeGreaterThan(0);

            // note: batch calls have the same function name but different overriden signature
            // which breaks in ethers, see https://github.com/ethers-io/ethers.js/issues/4383
            // that's why we need to specify which function we want to call instead of calling
            // blockProverContract.verifyAndSubmit() directly
            const verifyAndEmitSingle = blockProverContract.getFunction(
                'verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))',
            );
            await verifyAndEmitSingle(
                proofData.chainKey,
                proofData.headerNumber,
                proofData.txBytes,
                proofData.merkleProof,
                proofData.continuityProof,
                {
                    gasPrice: (await provider.getFeeData()).gasPrice,
                    gasLimit: 10000000,
                },
            );

            await forElapsedBlocks(api, { minBlocks: 3 });
        }, 45_000);

        it('graphQL returns known TransactionVerified entity', async () => {
            const response = await graphQLQuery(
                `query {
                    transactionVerifieds(
                        last: 1
                    ) { nodes {
                        id, chainId, height, transactionIndex, ccBlockNumber, timestamp
                    }}
                }`,
            );
            expect(response.data.transactionVerifieds.nodes).toBeTruthy();
            expect(response.data.transactionVerifieds.nodes.length).toEqual(1);

            for (const node of response.data.transactionVerifieds.nodes) {
                expect(node.id).toBeTruthy();

                expect(BigInt(node.chainId)).toEqual(BigInt(proofData.chainKey));
                expect(BigInt(node.height)).toEqual(BigInt(proofData.headerNumber));
                expect(BigInt(node.transactionIndex)).toBeGreaterThanOrEqual(0n);
                expect(BigInt(node.ccBlockNumber)).toBeGreaterThan(startingBlock);
                expect(BigInt(node.timestamp)).toBeGreaterThan(0);
                expect(BigInt(node.timestamp)).toBeLessThan(Date.now());
            }
        });
    });
});
