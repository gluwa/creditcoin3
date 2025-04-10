import { WebSocketProvider, ethers } from 'ethers';
import contractABIJSON = require('../artifacts/proof_verifier.json');
import validProof = require('../artifacts/valid_proof.json');
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { expectNoEventError, expectNoDispatchError } from '../../../lib';
import { u8aToHex } from '../../../lib/common';
import { fundFromSudo } from '../../integration-tests/helpers';
import { starkProgramHash, starkProgramVersion } from '../pallets/prover/consts';

const contractABI = contractABIJSON.contracts['sol/proof_verifier.sol:QueryVerifierContract'].abi;

describe('Precompile: get_result_segments()', (): void => {
    let contract: any;
    let alith: any;
    let api: ApiPromise;
    let queryId: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        await api.tx.sudo
            .sudo(api.tx.prover.setStarkProgramMetadata(starkProgramVersion, starkProgramHash))
            .signAndSend(root);

        const alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
        const query = {
            chainId: 0,
            height: 4,
            index: 0,
            layoutSegments: [
                {
                    offset: 0,
                    size: 681,
                },
            ],
        };
        const proof = u8aToHex(new TextEncoder().encode(JSON.stringify(validProof)));
        await api.tx.prover
            .submitProof(proof, query)
            .signAndSend(alice, { nonce: -1 }, ({ dispatchError, events, status }) => {
                expectNoDispatchError(api, dispatchError);
                if (events) events.forEach((event) => expectNoEventError(api, event));

                if (status.isInBlock) {
                    const querySubmitted = events.find(({ event: { method, section } }) => {
                        return section === 'prover' && method === 'QueryVerified';
                    });

                    expect(querySubmitted).toBeTruthy();
                    if (querySubmitted) {
                        queryId = querySubmitted.event.data[0];
                    }
                }
            });

        const evmProvider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

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
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('should work when called with valid query id', async () => {
        expect(queryId).toBeTruthy();
        const result = await contract.get_result_segments(queryId);
        expect(result.length).toBeGreaterThanOrEqual(1);

        const [offset, abiBytes] = result[0];
        expect(offset).toBeDefined();
        expect(abiBytes).toBeDefined();
    }, 30_000);

    test('should revert when called with invalid query id', async () => {
        await expect(contract.get_result_segments(new Uint8Array(32))).rejects.toThrow(
            /Result segments not found for query/,
        );
    }, 30_000);
});
