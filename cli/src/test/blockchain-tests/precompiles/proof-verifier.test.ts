import { WebSocketProvider, ethers } from 'ethers';
import contractABIJSON = require('../artifacts/proof_verifier.json');
import validProof = require('../artifacts/valid_proof.json');
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { u8aToHex } from '../../../lib/common';
import { fundFromSudo } from '../../integration-tests/helpers';
import { starkProgramHash, starkProgramVersion } from '../pallets/prover/consts';

const contractABI = contractABIJSON.contracts['sol/proof_verifier.sol:QueryVerifierContract'].abi;

describe('Precompile: verify()', (): void => {
    let contract: any;
    let evmProvider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        // need to call this, otherwise the call to submit_proof() which underlines the
        // precompile will fail
        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        await api.tx.sudo
            .sudo(api.tx.prover.setStarkProgramMetadata(starkProgramVersion, starkProgramHash))
            .signAndSend(root);

        evmProvider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        // precompile contract deployed at 3049 to hex, see runtime/src/precompiles.rs for more
        const precompileContractAddress = '0x0000000000000000000000000000000000000be9';

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, evmProvider);
        // will only work when connected to a chain locally and //Alice is root
        // either during local development or during runtime-upgrade against a fork
        // note: Alith starts with 2mil CTC during local development
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        // note: balances.Transfer is happy to accept Address20 directly too
        expect(result.status).toBe(0);

        contract = new ethers.Contract(precompileContractAddress, contractABI, alith);

        // note: larger timeout b/c this also executes against Testnet forks where block time is 15s
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await evmProvider.getFeeData()).gasPrice;
    });

    test('should work when called with valid input', async () => {
        const gasLimit = 20_000_000;

        const query = {
            chainId: 31337,
            height: 1,
            index: 0,
            layout: [
                {
                    offset: 0,
                    size: 418,
                },
            ],
        };
        // this needs to be a bytes array
        const proof = u8aToHex(new TextEncoder().encode(JSON.stringify(validProof)));

        const result = await contract.verify(proof, query, { gasPrice, gasLimit });
        const receipt = await result.wait();
        expect(receipt).toBeDefined();

        const txHash = result?.hash;
        expect(txHash).toBeDefined();
    }, 300_000);
});
