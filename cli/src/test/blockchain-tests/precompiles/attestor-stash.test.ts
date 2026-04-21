import { WebSocketProvider, ethers } from 'ethers';
import { decodeAddress } from '@polkadot/util-crypto';
import { u8aToHex } from '@polkadot/util';

import { newApi, ApiPromise, BN, MICROUNITS_PER_CTC } from '../../../lib';
import { evmAddressToSubstrateAddress } from '../../../lib/evm/address';
import { fundFromSudo, waitEras } from '../../integration-tests/helpers';
import { chain_Anvil2_Key } from '../pallets/supported-chains/consts';
import { attestorStashAddress } from './consts';
import { testIf } from '../../utils';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import contractABIJSON = require('../artifacts/attestor_stash.json');
const contractABI = contractABIJSON as unknown as ethers.InterfaceAbi;

// Event signatures (must match pallet-evm-precompile-attestor-stash)
const ATTESTOR_REGISTERED_TOPIC = ethers.id('AttestorRegistered(uint64,bytes32,address)');
const ATTESTOR_UNREGISTERED_TOPIC = ethers.id('AttestorUnregistered(uint64,bytes32,address)');
const ATTESTOR_CHILLED_TOPIC = ethers.id('AttestorChilled(uint64,bytes32,address)');
const UNBONDED_WITHDRAWN_TOPIC = ethers.id('UnbondedWithdrawn(address)');

// Encode a uint64 chain key as a 32-byte indexed topic (big-endian, zero-padded).
const chainKeyTopic = (key: number): string => '0x' + BigInt(key).toString(16).padStart(64, '0');

// Encode a 20-byte address as a 32-byte indexed topic (zero-left-padded).
const addressTopic = (addr: string): string => '0x' + addr.replace(/^0x/, '').toLowerCase().padStart(64, '0');

// Produce a random, well-formed 32-byte attestor id (bytes32).
const randomAttestorId = (): string => ethers.hexlify(ethers.randomBytes(32));

describe('Precompile: AttestorStash', (): void => {
    let contract: any;
    let provider: any;
    let alith: any;
    let api: ApiPromise;
    let gasPrice: bigint;

    // Alith's derived Substrate account (via HashedAddressMapping<BlakeTwo256>).
    // This is the `stash` AccountId used by pallet-attestation when the precompile
    // dispatches on behalf of the EVM caller.
    let stashSubstrateAddress: string;
    // The same account, encoded as the raw 32-byte public key hex ("0x..."). This
    // is the representation stored as the `stash` field of `AttestorPrimitivesAttestor`.
    let stashAccountIdHex: string;

    // Chain on which the attestor is registered.
    const chainKey = chain_Anvil2_Key;

    // The attestor id we register, chill, and unregister across the serial tests.
    const attestorId = randomAttestorId();

    beforeAll(async () => {
        ({ api } = await newApi((global as any).CREDITCOIN_API_URL));
        provider = new WebSocketProvider((global as any).CREDITCOIN_API_URL);

        const privateKey = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        alith = new ethers.Wallet(privateKey, provider);

        // Fund Alith's derived Substrate account (this is the `stash`) with plenty of
        // CTC. The default min bond is 100 CTC, so 2M is overkill.
        const result = await fundFromSudo(api, alith.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        expect(result.status).toBe(0);

        stashSubstrateAddress = evmAddressToSubstrateAddress(alith.address);
        stashAccountIdHex = u8aToHex(decodeAddress(stashSubstrateAddress));

        contract = new ethers.Contract(attestorStashAddress, contractABI, alith);
    }, 90_000);

    afterAll(async () => {
        await api.disconnect();
    });

    beforeEach(async () => {
        gasPrice = (await provider.getFeeData()).gasPrice;
    });

    // -----------------------------------------------------------------------------
    // registerAttestor
    // -----------------------------------------------------------------------------

    test('registerAttestor succeeds, creates Attestors storage entry, and emits AttestorRegistered', async () => {
        // Sanity: no attestor with this id registered yet on this chain.
        const before = await api.query.attestation.attestors(chainKey, attestorId);
        expect(before.isSome).toBe(false);

        const tx = await contract.registerAttestor(chainKey, attestorId, { gasPrice });
        const receipt = await tx.wait();
        expect(receipt.status).toBe(1);

        // Storage updated: `Attestors[chainKey][attestorId]` now points at Alith's derived stash.
        const after = await api.query.attestation.attestors(chainKey, attestorId);
        expect(after.isSome).toBe(true);
        const storedAttestor = after.unwrap();
        expect(storedAttestor.stash.toHex().toLowerCase()).toBe(stashAccountIdHex.toLowerCase());

        // Log topic/arg assertions.
        const log = receipt.logs.find(
            (l: any) =>
                l.address.toLowerCase() === attestorStashAddress.toLowerCase() &&
                l.topics[0] === ATTESTOR_REGISTERED_TOPIC,
        );
        expect(log).toBeDefined();
        expect(log.topics[1]).toBe(chainKeyTopic(chainKey));
        expect(log.topics[2]).toBe(attestorId.toLowerCase());
        expect(log.topics[3]).toBe(addressTopic(alith.address));
    }, 90_000);

    test('registerAttestor twice for the same stash on the same chain reverts', async () => {
        await expect(contract.registerAttestor(chainKey, attestorId, { gasPrice })).rejects.toThrow(
            /Dispatched call failed with error:/,
        );
    });

    test('registerAttestor on an unsupported chain reverts', async () => {
        const unsupportedChainKey = 9_999_999;
        // Also use a fresh attestor id: this mustn't conflict with the one we just registered.
        const freshAttestorId = randomAttestorId();

        await expect(contract.registerAttestor(unsupportedChainKey, freshAttestorId, { gasPrice })).rejects.toThrow(
            /Dispatched call failed with error:/,
        );
    });

    // -----------------------------------------------------------------------------
    // chill
    // -----------------------------------------------------------------------------

    test('chill on an unknown attestor id reverts (AddressNotAttestor)', async () => {
        const unknownAttestorId = randomAttestorId();
        await expect(contract.chill(chainKey, unknownAttestorId, { gasPrice })).rejects.toThrow(
            /Dispatched call failed with error:/,
        );
    });

    test('chill succeeds and emits AttestorChilled for the caller stash', async () => {
        const tx = await contract.chill(chainKey, attestorId, { gasPrice });
        const receipt = await tx.wait();
        expect(receipt.status).toBe(1);

        const log = receipt.logs.find(
            (l: any) =>
                l.address.toLowerCase() === attestorStashAddress.toLowerCase() &&
                l.topics[0] === ATTESTOR_CHILLED_TOPIC,
        );
        expect(log).toBeDefined();
        expect(log.topics[1]).toBe(chainKeyTopic(chainKey));
        expect(log.topics[2]).toBe(attestorId.toLowerCase());
        expect(log.topics[3]).toBe(addressTopic(alith.address));
    }, 60_000);

    // -----------------------------------------------------------------------------
    // unregisterAttestor
    // -----------------------------------------------------------------------------

    test('unregisterAttestor succeeds, removes Attestors storage entry, and emits AttestorUnregistered', async () => {
        const tx = await contract.unregisterAttestor(chainKey, attestorId, { gasPrice });
        const receipt = await tx.wait();
        expect(receipt.status).toBe(1);

        // Storage entry for `Attestors[chainKey][attestorId]` is gone.
        const after = await api.query.attestation.attestors(chainKey, attestorId);
        expect(after.isSome).toBe(false);

        const log = receipt.logs.find(
            (l: any) =>
                l.address.toLowerCase() === attestorStashAddress.toLowerCase() &&
                l.topics[0] === ATTESTOR_UNREGISTERED_TOPIC,
        );
        expect(log).toBeDefined();
        expect(log.topics[1]).toBe(chainKeyTopic(chainKey));
        expect(log.topics[2]).toBe(attestorId.toLowerCase());
        expect(log.topics[3]).toBe(addressTopic(alith.address));
    }, 60_000);

    test('unregisterAttestor a second time reverts (AddressNotAttestor)', async () => {
        await expect(contract.unregisterAttestor(chainKey, attestorId, { gasPrice })).rejects.toThrow(
            /Dispatched call failed with error:/,
        );
    });

    // -----------------------------------------------------------------------------
    // withdrawUnbonded
    //
    // Only meaningful once `bondingDuration` eras have elapsed since unregister.
    // We run this behind an env flag because waiting for the unbond period can
    // take several minutes on a local dev chain and isn't appropriate for every
    // CI job.
    // -----------------------------------------------------------------------------

    testIf(
        process.env.RUN_WITHDRAW_UNBONDED !== undefined,
        'withdrawUnbonded succeeds after bonding duration and emits UnbondedWithdrawn',
        async () => {
            const unbondingPeriod: number = api.consts.attestation.bondingDuration.toNumber();
            await waitEras(unbondingPeriod + 1, api);

            // Refresh `gasPrice` now that several minutes have elapsed: the
            // EIP-1559 base fee cached in `beforeEach` has drifted and submitting
            // with the stale value yields "gas price less than block base fee".
            gasPrice = (await provider.getFeeData()).gasPrice;

            const tx = await contract.withdrawUnbonded({ gasPrice });
            const receipt = await tx.wait();
            expect(receipt.status).toBe(1);

            const log = receipt.logs.find(
                (l: any) =>
                    l.address.toLowerCase() === attestorStashAddress.toLowerCase() &&
                    l.topics[0] === UNBONDED_WITHDRAWN_TOPIC,
            );
            expect(log).toBeDefined();
            expect(log.topics[1]).toBe(addressTopic(alith.address));
        },
        600_000,
    );
});
