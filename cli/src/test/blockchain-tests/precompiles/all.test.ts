import { WebSocketProvider, ethers, parseEther } from 'ethers';
import transferABIJSON = require('../artifacts/substrate_transfer.json');
import verifierABIJSON = require('../artifacts/proof_verifier.json');
import { Keyring } from '@polkadot/keyring';
import { mnemonicGenerate } from '@polkadot/util-crypto';
import { expectNoEventError, expectNoDispatchError } from '../../../lib';
import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { fundFromSudo } from '../../integration-tests/helpers';

const transferABI = transferABIJSON.contracts['sol/substrate_transfer.sol:SubstrateTransfer'].abi;
const verifierABI = verifierABIJSON.contracts['sol/proof_verifier.sol:QueryVerifierContract'].abi;

import { ContractTransactionResponse } from 'ethers';
import validProof = require('../artifacts/valid_proof.json');
import { validQuery } from '../helpers';
import { u8aToHex } from '../../../lib/common';
import { starkProgramHash, starkProgramVersion } from '../pallets/prover/consts';

describe('Precompile: get_result_segments()', (): void => {
    let contract: any;
    let alith: any;
    let api: ApiPromise;
    let queryId: any;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));

        const root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        let nonce = await api.rpc.system.accountNextIndex(root.address);
        await api.tx.sudo
            .sudo(api.tx.prover.setStarkProgramMetadata(starkProgramVersion, starkProgramHash))
            .signAndSend(root, { nonce });

        const alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');
        const proof = u8aToHex(new TextEncoder().encode(JSON.stringify(validProof)));
        nonce = await api.rpc.system.accountNextIndex(alice.address);
        await api.tx.prover
            .submitProof(proof, validQuery)
            .signAndSend(alice, { nonce }, ({ dispatchError, events, status }) => {
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

        contract = new ethers.Contract(precompileContractAddress, verifierABI, alith);
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

    test('should error when called with invalid query id', async () => {
        await expect(contract.get_result_segments(new Uint8Array(32))).rejects.toThrow(
            /missing revert data.*code=CALL_EXCEPTION/,
        );
    }, 30_000);
});

describe('Precompile: transfer_substrate()', (): void => {
    let contract: any;
    let destination: any;
    let destinationBalanceBefore: bigint;
    let provider: any;
    let alith: any;
    let alithBalanceBefore: bigint;
    let api: ApiPromise;
    let gasPrice: bigint;
    let gasLimit: number;

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        // precompile contract deployed at 4049 to hex, see runtime/src/precompiles.rs for more
        const precompileContractAddress = '0x0000000000000000000000000000000000000fd1';

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);
        // will only work when connected to a chain locally and //Alice is root
        // either during local development or during runtime-upgrade against a fork
        // note: Alith starts with 2mil CTC during local development
        const result = await fundFromSudo(alith.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        // note: balances.Transfer is happy to accept Address20 directly too
        expect(result.status).toBe(0);
        alithBalanceBefore = await provider.getBalance(alith.address);

        contract = new ethers.Contract(precompileContractAddress, transferABI, alith);
        const target = new Keyring();
        destination = target.addFromMnemonic(mnemonicGenerate());

        destinationBalanceBefore = (await api.derive.balances.all(destination.address)).availableBalance.toBigInt();

        gasLimit = 10000000;
        // note: larger timeout b/c this also executes against Testnet forks where block time is 15s
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
    });

    test('should work when caller has enough funds', async () => {
        const amount = parseEther('10.0');
        const result = await contract.transfer_substrate(destination.addressRaw, amount, {
            gasPrice,
            gasLimit,
        });
        const receipt = await result.wait();
        expect(receipt).toBeDefined();

        const txHash = result?.hash;
        expect(txHash).toBeDefined();

        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        expect(alithBalanceBefore).toBe(alithBalanceAfter + amount + BigInt(receipt.cumulativeGasUsed * gasPrice));

        const destinationBalanceAfter = (
            await api.derive.balances.all(destination.address)
        ).availableBalance.toBigInt();
        expect(destinationBalanceAfter).toBe(destinationBalanceBefore + BigInt(amount));
    });

    test('should fail when sending more than total issuance', async () => {
        const totalIssuance = (await api.query.balances.totalIssuance()).toBigInt();
        // trying to send 1 bil more than total issuance
        const amount = totalIssuance + BigInt(1_000_000_000_000_000_000_000_000_000);

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/Dispatched call failed with error: Arithmetic\(Underflow\)/);
        // ^^^ appears to come from can_withdraw()
        // ^^^ appears to come from do_transfer_reserved()
        // https://github.com/paritytech/polkadot-sdk/blob/698d9ae5b32785d3a5a55b770e973bbdb59ad271/substrate/frame/balances/src/impl_fungible.rs#L113

        // Alice may have paid gas fees regardless of the error
        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        expect(alithBalanceAfter).toBeLessThanOrEqual(alithBalanceBefore);
    });

    test('should fail when sending more than available funds', async () => {
        // trying to send 1 mil more than available balance
        const amount = alithBalanceBefore + BigInt(1_000_000_000_000_000_000_000_000);

        await expect(
            contract.transfer_substrate(destination.addressRaw, amount, {
                gasPrice,
            }),
        ).rejects.toThrow(/execution reverted:.*Dispatched call failed with error: Token\(FundsUnavailable\)/);
        // ^^^ appears to come from can_withdraw()
        // ^^^ appears to come from do_transfer_reserved()
        // https://github.com/paritytech/polkadot-sdk/blob/698d9ae5b32785d3a5a55b770e973bbdb59ad271/substrate/frame/balances/src/impl_fungible.rs#L113

        // Alice may have paid gas fees regardless of the error
        const alithBalanceAfter: bigint = await provider.getBalance(alith.address);
        expect(alithBalanceAfter).toBeLessThanOrEqual(alithBalanceBefore);
    });
});

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
        const nonce = await api.rpc.system.accountNextIndex(root.address);
        await api.tx.sudo
            .sudo(api.tx.prover.setStarkProgramMetadata(starkProgramVersion, starkProgramHash))
            .signAndSend(root, { nonce });

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

        contract = new ethers.Contract(precompileContractAddress, verifierABI, alith);

        // note: larger timeout b/c this also executes against Testnet forks where block time is 15s
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await evmProvider.getFeeData()).gasPrice;
    });

    test('should work when called with valid input', async () => {
        const gasLimit = 30_000_000;

        // this needs to be a bytes array
        const proof = u8aToHex(new TextEncoder().encode(JSON.stringify(validProof)));

        // when passing this to the verify() precompile it expects the field to be called `layout`
        // while the extrinsic expects this as `layoutSegments`
        const query = {};
        // @ts-ignore
        delete Object.assign(query, validQuery, { ['layout']: validQuery.layoutSegments }).layoutSegments;

        const result = await contract.verify(proof, query, { gasPrice, gasLimit });
        const receipt = await result.wait();
        expect(receipt).toBeDefined();

        const txHash = result?.hash;
        expect(txHash).toBeDefined();
    }, 300_000);

    test('should revert when called with invalid input', async () => {
        const gasLimit = 30_000_000;

        // this needs to be a bytes array
        const proof = new Uint8Array(32);

        // when passing this to the verify() precompile it expects the field to be called `layout`
        // while the extrinsic expects this as `layoutSegments`
        const query = {};
        // @ts-ignore
        delete Object.assign(query, validQuery, { ['layout']: validQuery.layoutSegments }).layoutSegments;

        await expect(
            contract.verify(proof, query, { gasPrice, gasLimit }).then((tx: ContractTransactionResponse) => tx.wait()),
        ).rejects.toThrow(/reverted/);
    }, 300_000);
});
