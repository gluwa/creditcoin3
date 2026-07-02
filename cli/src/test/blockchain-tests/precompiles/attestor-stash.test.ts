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
    // PoV-heavy calls need an explicit gas limit; auto-estimate under-counts the
    // proof-size component (5 KB @ ratio 14.3 → ~72 K PoV gas alone).
    const GAS_LIMIT = 1_000_000;

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

        const tx = await contract.registerAttestor(chainKey, attestorId, { gasPrice, gasLimit: GAS_LIMIT });
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
        await expect(contract.registerAttestor.staticCall(chainKey, attestorId)).rejects.toThrow(
            /Dispatched call failed with error:.*AlreadyAttestor/,
        );
    });

    test('registerAttestor on an unsupported chain reverts', async () => {
        const unsupportedChainKey = 9_999_999;
        // Also use a fresh attestor id: this mustn't conflict with the one we just registered.
        const freshAttestorId = randomAttestorId();

        await expect(contract.registerAttestor.staticCall(unsupportedChainKey, freshAttestorId)).rejects.toThrow(
            /Dispatched call failed with error:.*ChainNotSupported/,
        );
    });

    // -----------------------------------------------------------------------------
    // getLedger / getLedgerByAddress / getCallerLedger
    //
    // Audit follow-up: `getLedger(bytes32)` expects the *hashed* AccountId32 that
    // AddressMapping derives from the EVM address, which is easy to misuse — EVM
    // consumers tend to convert the emitted `address` to bytes32 and get an empty
    // ledger back. The two new entries surface the same data keyed by the EVM
    // `address`, so we cross-check that all three return the same ledger once one
    // exists for the caller. The `LedgerInfo.stash` field is what proves they
    // refer to the *same account* — without it, three distinct ledgers carrying
    // identical balances would be indistinguishable.
    // -----------------------------------------------------------------------------

    test('getLedger / getLedgerByAddress / getCallerLedger all return the same ledger', async () => {
        const minBond = (await contract.getMinBondRequirement(chainKey)) as bigint;
        expect(minBond).toBeGreaterThan(0n);

        // `getLedger` keyed by the *hashed* AccountId32.
        const byStash = await contract.getLedger(stashAccountIdHex);
        // `getLedgerByAddress` keyed by the EVM address (precompile applies
        // AddressMapping internally) — must resolve to the same stash.
        const byAddress = await contract.getLedgerByAddress(alith.address);
        // `getCallerLedger()` uses msg.sender so the result should equal `byAddress`.
        const byCaller = await contract.getCallerLedger();

        // The stash actually has a ledger after registerAttestor above.
        expect(byStash.exists).toBe(true);
        expect(byStash.totalStaked).toBe(minBond);
        expect(byStash.active).toBe(minBond);
        expect(byStash.unlockingChunks).toBe(0n);
        expect(byStash.withdrawable).toBe(0n);

        // The identity proof: the ledger's `stash` field must be Alith's hashed
        // AccountId32 — the same value `getLedger` was keyed by. This ties the
        // EVM-address entries to a concrete on-chain account rather than just a
        // bag of matching balance fields.
        expect(byStash.stash.toLowerCase()).toBe(stashAccountIdHex.toLowerCase());
        expect(byAddress.stash.toLowerCase()).toBe(stashAccountIdHex.toLowerCase());
        expect(byCaller.stash.toLowerCase()).toBe(stashAccountIdHex.toLowerCase());

        // Full equality across all three entries, including the stash field. Any
        // divergence would re-introduce the silently-empty-ledger foot-gun the new
        // entries were added to prevent.
        expect(byAddress.stash).toBe(byStash.stash);
        expect(byAddress.exists).toBe(byStash.exists);
        expect(byAddress.totalStaked).toBe(byStash.totalStaked);
        expect(byAddress.active).toBe(byStash.active);
        expect(byAddress.unlockingChunks).toBe(byStash.unlockingChunks);
        expect(byAddress.withdrawable).toBe(byStash.withdrawable);

        expect(byCaller.stash).toBe(byStash.stash);
        expect(byCaller.exists).toBe(byStash.exists);
        expect(byCaller.totalStaked).toBe(byStash.totalStaked);
        expect(byCaller.active).toBe(byStash.active);
        expect(byCaller.unlockingChunks).toBe(byStash.unlockingChunks);
        expect(byCaller.withdrawable).toBe(byStash.withdrawable);

        // Negative control: a different, unregistered address must resolve to a
        // distinct (empty) stash — its ledger does not exist and carries the zero
        // stash, so it can never be confused with Alith's.
        const otherWallet = ethers.Wallet.createRandom();
        const otherByAddress = await contract.getLedgerByAddress(otherWallet.address);
        expect(otherByAddress.exists).toBe(false);
        expect(otherByAddress.stash).not.toBe(byStash.stash);
    });

    // -----------------------------------------------------------------------------
    // chill
    // -----------------------------------------------------------------------------

    test('chill on an unknown attestor id reverts (AddressNotAttestor)', async () => {
        const unknownAttestorId = randomAttestorId();
        await expect(contract.chill.staticCall(chainKey, unknownAttestorId)).rejects.toThrow(
            /Dispatched call failed with error:.*AddressNotAttestor/,
        );
    });

    test('chill on a registered but idle attestor reverts (AttestorAlreadyIdle)', async () => {
        await expect(contract.chill.staticCall(chainKey, attestorId)).rejects.toThrow(
            /Dispatched call failed with error:.*AttestorAlreadyIdle/,
        );
    }, 60_000);

    // -----------------------------------------------------------------------------
    // unregisterAttestor
    // -----------------------------------------------------------------------------

    test('unregisterAttestor succeeds, removes Attestors storage entry, and emits AttestorUnregistered', async () => {
        const tx = await contract.unregisterAttestor(chainKey, attestorId, { gasPrice, gasLimit: GAS_LIMIT });
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
        await expect(contract.unregisterAttestor.staticCall(chainKey, attestorId)).rejects.toThrow(
            /Dispatched call failed with error:.*AddressNotAttestor/,
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

            const tx = await contract.withdrawUnbonded({ gasPrice, gasLimit: GAS_LIMIT });
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
