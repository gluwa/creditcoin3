import { WebSocketProvider, ethers } from 'ethers';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';
import { chain_Anvil2_Key } from '../pallets/supported-chains/consts';
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

        // We expect 6 supported chains as per the current genesis configuration
        expect(supportedChains.length).toEqual(6);
    });

    test('get_chain_by_key should return correct chain info', async () => {
        // Check with supported chain key
        const supportedChain = await contract.get_chain_by_key(supportedChainKey, { gasPrice, gasLimit });
        expect(supportedChain).toBeDefined();
        // We expect the chain to exist
        expect(supportedChain.exists).toEqual(true);

        // Check with non supported chain key
        const unknownChain = await contract.get_chain_by_key(unknownChainKey, { gasPrice, gasLimit });
        expect(unknownChain).toBeDefined();
        // We expect the chain to not exist
        expect(unknownChain.exists).toEqual(false);
    });

    test('get_latest_attestation_height_and_hash should return data', async () => {
        const latestAttestation = await contract.get_latest_attestation_height_and_hash(supportedChainKey, {
            gasPrice,
            gasLimit,
        });
        expect(latestAttestation).toBeDefined();
        // Since we have no attestations, we should get default values
        expect(latestAttestation.height).toEqual(0n);
        expect(latestAttestation.hash).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(latestAttestation.exists).toEqual(false);
    });

    test('get_latest_checkpoint_height_and_hash should return data', async () => {
        const latestCheckpoint = await contract.get_latest_checkpoint_height_and_hash(supportedChainKey, {
            gasPrice,
            gasLimit,
        });
        expect(latestCheckpoint).toBeDefined();
        // Since we have no checkpoints, we should get default values
        expect(latestCheckpoint.height).toEqual(0n);
        expect(latestCheckpoint.hash).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(latestCheckpoint.exists).toEqual(false);
    });

    test('find_highest_attested_before should return data', async () => {
        const highestAttested = await contract.find_highest_attested_before(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(highestAttested).toBeDefined();
        // Since we have no attestations, we should get default values
        expect(highestAttested.height).toEqual(0n);
        expect(highestAttested.hash).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(highestAttested.exists).toEqual(false);
    });

    test('find_lowest_attested_after should return data', async () => {
        const lowestAttested = await contract.find_lowest_attested_after(supportedChainKey, targetHeight, {
            gasPrice,
            gasLimit,
        });
        expect(lowestAttested).toBeDefined();
        // Since we have no attestations, we should get default values
        expect(lowestAttested.height).toEqual(0n);
        expect(lowestAttested.hash).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(lowestAttested.exists).toEqual(false);
    });

    test('is_height_attested should return data', async () => {
        const isAttested = await contract.is_height_attested(supportedChainKey, targetHeight, { gasPrice, gasLimit });
        expect(isAttested).toBeDefined();
        // Since we have no attestations, we expect false
        expect(isAttested).toEqual(false);
    });

    test('get_attestation_bounds should return data', async () => {
        const bounds = await contract.get_attestation_bounds(supportedChainKey, targetHeight, { gasPrice, gasLimit });
        expect(bounds).toBeDefined();
        // Since we have no attestations, we should get default values
        expect(bounds.parentHeight).toEqual(0n);
        expect(bounds.parentHash).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(bounds.parentIsAttestation).toEqual(false);
        expect(bounds.childHeight).toEqual(0n);
        expect(bounds.childHash).toEqual('0x0000000000000000000000000000000000000000000000000000000000000000');
        expect(bounds.childIsAttestation).toEqual(false);
        expect(bounds.isAttested).toEqual(false);
    });
});
