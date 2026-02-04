import { WebSocketProvider, ethers } from 'ethers';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';
import {
    chain_Anvil2_Key,
    chain_Anvil2_Id,
    chain_Anvil2_Name_Hex,
    encoding_version_1,
} from '../pallets/supported-chains/consts';
import { chainInfoAddress } from './consts';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('../artifacts/chain_info.json');
const contractABI = contractABIJSON as unknown as ethers.InterfaceAbi;

const supportedChainKey = chain_Anvil2_Key;
const unknownChainKey = 42732;

const targetHeight = 1000;

describe('Precompile: ChainInfo', (): void => {
    let contract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;
    let gasLimit: number;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);
        // will only work when connected to a chain locally and //Alice is root
        // either during local development or during runtime-upgrade against a fork
        // note: Alith starts with 2mil CTC during local development
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        // note: balances.Transfer is happy to accept Address20 directly too
        expect(result.status).toBe(0);

        contract = new ethers.Contract(chainInfoAddress, contractABI, alith);

        gasLimit = 10000000;
        // note: larger timeout b/c this also executes against Testnet forks where block time is 15s
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
    });

    test('get_supported_chains should return a list of supported chains', async () => {
        const supportedChains = await contract.get_supported_chains({ gasPrice, gasLimit });
        expect(supportedChains).toBeDefined();
        expect(Array.isArray(supportedChains)).toBe(true);

        // We check that each chain info entry has the expected tuple elements
        for (const chainInfo of supportedChains) {
            expect(Array.isArray(chainInfo)).toBe(true);
            expect(chainInfo.length).toBe(4);

            expect(typeof chainInfo[0]).toBe('bigint'); // Chain Key
            expect(typeof chainInfo[1]).toBe('bigint'); // Chain Id
            expect(typeof chainInfo[2]).toBe('string'); // Chain Name
            expect(typeof chainInfo[3]).toBe('bigint'); // Chain Encoding
        }

        // We expect 6 supported chains as per the current genesis configuration
        expect(supportedChains.length).toEqual(6);
    });

    test('get_chain_by_key should return correct chain info', async () => {
        // Check with supported chain key
        const supportedChainResult = await contract.get_chain_by_key(supportedChainKey, { gasPrice, gasLimit });

        // We should get a two element tuple
        expect(supportedChainResult).toBeDefined();
        expect(Array.isArray(supportedChainResult)).toBe(true);
        expect(supportedChainResult.length).toBe(2);

        // First element is the 'chain' attribute, which is a tuple of 4 elements
        expect(typeof supportedChainResult[0]).toBe('object');
        expect(Array.isArray(supportedChainResult[0])).toBe(true); // chain property
        expect(typeof supportedChainResult[1]).toBe('boolean'); // exists property
        expect(supportedChainResult[1]).toEqual(true); // exists should be true for supported chain

        const chainTuple = supportedChainResult[0];
        expect(chainTuple.length).toBe(4);

        // Validate each element of the chain tuple
        expect(typeof chainTuple[0]).toBe('bigint'); // Chain Key
        expect(chainTuple[0]).toEqual(BigInt(supportedChainKey));
        expect(typeof chainTuple[1]).toBe('bigint'); // Chain Id
        expect(chainTuple[1]).toEqual(BigInt(chain_Anvil2_Id));
        expect(typeof chainTuple[2]).toBe('string'); // Chain Name
        expect(chainTuple[2]).toEqual(chain_Anvil2_Name_Hex); // 'Anvil2' in hex
        expect(typeof chainTuple[3]).toBe('bigint'); // Chain Encoding
        expect(chainTuple[3]).toEqual(BigInt(encoding_version_1));

        // Check with non supported chain key
        const unknownChain = await contract.get_chain_by_key(unknownChainKey, { gasPrice, gasLimit });
        expect(unknownChain).toBeDefined();
        // We expect the chain to not exist
        expect(unknownChain.exists).toEqual(false);
    });

    test('get_latest_attestation_height_and_hash should return data', async () => {
        const latestAttestationResult = await contract.get_latest_attestation_height_and_hash(supportedChainKey, {
            gasPrice,
            gasLimit,
        });
        expect(latestAttestationResult).toBeDefined();
        expect(Array.isArray(latestAttestationResult)).toBe(true);
        expect(latestAttestationResult.length).toBe(4);
        // Since we have no attestations, we should get default values
        expect(typeof latestAttestationResult[0]).toBe('bigint'); // height
        expect(latestAttestationResult[0]).toEqual(0n);
        expect(typeof latestAttestationResult[1]).toBe('string'); // hash
        expect(latestAttestationResult[1]).toEqual(
            '0x0000000000000000000000000000000000000000000000000000000000000000',
        );
        expect(typeof latestAttestationResult[2]).toBe('boolean'); // isAttestation
        expect(latestAttestationResult[2]).toEqual(false);
        expect(typeof latestAttestationResult[3]).toBe('boolean'); // exists
        expect(latestAttestationResult[3]).toEqual(false);
    });

    test('get_latest_checkpoint_height_and_hash should return data', async () => {
        const latestCheckpointResult = await contract.get_latest_checkpoint_height_and_hash(supportedChainKey, {
            gasPrice,
            gasLimit,
        });
        expect(latestCheckpointResult).toBeDefined();
        expect(Array.isArray(latestCheckpointResult)).toBe(true);
        expect(latestCheckpointResult.length).toBe(4);
        // Since we have no checkpoints, we should get default values
        expect(typeof latestCheckpointResult[0]).toBe('bigint'); // height
        expect(latestCheckpointResult[0]).toEqual(0n);
        expect(typeof latestCheckpointResult[1]).toBe('string'); // hash
        expect(latestCheckpointResult[1]).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(typeof latestCheckpointResult[2]).toBe('boolean'); // isAttestation
        expect(latestCheckpointResult[2]).toEqual(false);
        expect(typeof latestCheckpointResult[3]).toBe('boolean'); // exists
        expect(latestCheckpointResult[3]).toEqual(false);
    });

    test('find_highest_attested_before should return data', async () => {
        const highestAttestedResult = await contract.find_highest_attested_before(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(highestAttestedResult).toBeDefined();
        expect(Array.isArray(highestAttestedResult)).toBe(true);
        expect(highestAttestedResult.length).toBe(4);
        // Since we have no attestations, we should get default values
        expect(typeof highestAttestedResult[0]).toBe('bigint'); // height
        expect(highestAttestedResult[0]).toEqual(0n);
        expect(typeof highestAttestedResult[1]).toBe('string'); // hash
        expect(highestAttestedResult[1]).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(typeof highestAttestedResult[2]).toBe('boolean'); // isAttestation
        expect(highestAttestedResult[2]).toEqual(false);
        expect(typeof highestAttestedResult[3]).toBe('boolean'); // exists
        expect(highestAttestedResult[3]).toEqual(false);
    });

    test('find_lowest_attested_after should return data', async () => {
        const lowestAttestedResult = await contract.find_lowest_attested_after(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(lowestAttestedResult).toBeDefined();
        expect(Array.isArray(lowestAttestedResult)).toBe(true);
        expect(lowestAttestedResult.length).toBe(4);
        // Since we have no attestations, we should get default values
        expect(typeof lowestAttestedResult[0]).toBe('bigint'); // height
        expect(lowestAttestedResult[0]).toEqual(0n);
        expect(typeof lowestAttestedResult[1]).toBe('string'); // hash
        expect(lowestAttestedResult[1]).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(typeof lowestAttestedResult[2]).toBe('boolean'); // isAttestation
        expect(lowestAttestedResult[2]).toEqual(false);
        expect(typeof lowestAttestedResult[3]).toBe('boolean'); // exists
        expect(lowestAttestedResult[3]).toEqual(false);
    });

    test('is_height_attested should return data', async () => {
        const isAttestedResult = await contract.is_height_attested(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(typeof isAttestedResult).toBe('boolean');
        // Since we have no attestations, we expect false
        expect(isAttestedResult).toEqual(false);
    });

    test('get_attestation_bounds should return data', async () => {
        const boundsResult = await contract.get_attestation_bounds(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(boundsResult).toBeDefined();
        expect(Array.isArray(boundsResult)).toBe(true);
        expect(boundsResult.length).toBe(7);
        // Since we have no attestations, we should get default values
        expect(typeof boundsResult[0]).toBe('bigint'); // parentHeight
        expect(boundsResult[0]).toEqual(0n);
        expect(typeof boundsResult[1]).toBe('string'); // parentHash
        expect(boundsResult[1]).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(typeof boundsResult[2]).toBe('boolean'); // parentIsAttestation
        expect(boundsResult[2]).toEqual(false);
        expect(typeof boundsResult[3]).toBe('bigint'); // childHeight
        expect(boundsResult[3]).toEqual(0n);
        expect(typeof boundsResult[4]).toBe('string'); // childHash
        expect(boundsResult[4]).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(typeof boundsResult[5]).toBe('boolean'); // childIsAttestation
        expect(boundsResult[5]).toEqual(false);
        expect(typeof boundsResult[6]).toBe('boolean'); // isAttested
        expect(boundsResult[6]).toEqual(false);
    });

    test('get_attestation_height_for_digest should return data', async () => {
        const targetHash = '0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef';
        const heightByDigest = await contract.get_attestation_height_for_digest(supportedChainKey, targetHash, {
            gasPrice,
            gasLimit,
        });
        expect(heightByDigest).toBeDefined();
        // We expect a 2 element tuple
        expect(Array.isArray(heightByDigest)).toEqual(true);
        expect(heightByDigest.length).toEqual(2);
        // Since we have no attestations, we should get default values
        expect(typeof heightByDigest[0]).toBe('bigint'); // height
        expect(heightByDigest[0]).toEqual(0n);
        expect(typeof heightByDigest[1]).toBe('boolean'); // exists
        expect(heightByDigest[1]).toEqual(false);
    });

    test('get_checkpoint_for_height should return data', async () => {
        const digestByHeight = await contract.get_checkpoint_for_height(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(digestByHeight).toBeDefined();
        // We expect a 2 element tuple
        expect(Array.isArray(digestByHeight)).toEqual(true);
        expect(digestByHeight.length).toEqual(2);
        // Since we have no checkpoints, we should get default values
        expect(typeof digestByHeight[0]).toBe('string'); // hash
        expect(digestByHeight[0]).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(typeof digestByHeight[1]).toBe('boolean'); // exists
        expect(digestByHeight[1]).toEqual(false);
    });
});
