import { Keyring } from '@polkadot/keyring';
import { BN } from '@polkadot/util';
import { blake2AsU8a, cryptoWaitReady, decodeAddress, mnemonicGenerate } from '@polkadot/util-crypto';
import { ethers, getBytes, hexlify, JsonRpcProvider, parseEther, zeroPadValue, ContractFactory } from 'ethers';

import { newApi, ApiPromise, KeyringPair } from '../../../lib';
import { chain_Anvil1_Key } from '../pallets/supported-chains/consts';
import type { SubmittableExtrinsic } from '@polkadot/api/types';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import tokenArtifact = require('../artifacts/MockAttestToken.json');
// eslint-disable-next-line @typescript-eslint/no-require-imports
import precompileAbi = require('../artifacts/attest_coin_precompile.json');
import { forElapsedBlocks } from '../../utils';

const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';

/** Headroom for ERC-20 subcall + precompile `mint` dispatch (runtime avoids double-counting PoV vs `try_dispatch`). */
const DEPOSIT_PRECOMPILE_GAS = 8_000_000n;

/** Matches `pallet_evm::HashedAddressMapping::<BlakeTwo256>::into_account_id` (prefix `evm:` + 20-byte address). */
function substrateAccountIdFromEvmAddress(evmAddress: string): Uint8Array {
    const addr = getBytes(evmAddress);
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(addr, 4);
    return blake2AsU8a(payload, 256);
}

/** `bytes32` for precompile `depositTo` / Solidity — 32-byte raw Substrate `AccountId`. */
function accountIdToBytes32(accountSs58OrRaw: string | Uint8Array): string {
    const raw = typeof accountSs58OrRaw === 'string' ? decodeAddress(accountSs58OrRaw) : accountSs58OrRaw;
    return zeroPadValue(hexlify(raw), 32);
}

/**
 * Foundry Anvil account #0 — always pre-funded. Same key as `attestor/scripts/Transfer.js` (`getSigner`).
 * Alith / `CREDITCOIN_EVM_PRIVATE_KEY('alice')` is **not** funded on vanilla Anvil.
 */
const ANVIL_DEFAULT_ACCOUNT_0_PRIVATE_KEY =
    '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

/** Foundry Anvil / local EVM — default matches CI `anvil --port 8141`; override with `ANVIL1_HTTP_URL` (e.g. `http://127.0.0.1:8545`). */
function anvilHttpUrl(): string {
    return process.env.ANVIL1_HTTP_URL ?? 'http://127.0.0.1:8141';
}

/** Creditcoin node HTTP JSON-RPC for the **embedded EVM** (precompile + `setAttestCoinToken` target). */
function creditcoinHttpUrl(): string {
    if (process.env.CREDITCOIN_HTTP_URL) {
        return process.env.CREDITCOIN_HTTP_URL;
    }
    const ws = process.env.CREDITCOIN_API_URL ?? (global as any).CREDITCOIN_API_URL;
    if (typeof ws === 'string' && ws.startsWith('ws')) {
        return ws.replace(/^ws/, 'http');
    }
    return 'http://127.0.0.1:9944';
}

/** Set `ATTEST_COIN_REWARDS_DEBUG=1` when running jest (without `--silent`) for step logs. */
function dbg(...args: unknown[]) {
    if (process.env.ATTEST_COIN_REWARDS_DEBUG === '1') {
        // eslint-disable-next-line no-console
        console.error('[attest-coin-rewards]', ...args);
    }
}

/** `chain_key` in claim preimage — Anvil1 matches CI zombienet `--chain-key=2`. */
function toLeU64(n: bigint): Buffer {
    let v = n;
    const b = Buffer.alloc(8);
    for (let i = 0; i < 8; i++) {
        b[i] = Number(v & 0xffn);
        v >>= 8n;
    }
    return b;
}

function toLeU128(n: bigint): Buffer {
    let v = n;
    const b = Buffer.alloc(16);
    for (let i = 0; i < 16; i++) {
        b[i] = Number(v & 0xffn);
        v >>= 8n;
    }
    return b;
}

/** Must match `pallet_attest_coin_rewards::Pallet::claim_signing_message`. */
function buildClaimSigningMessage(
    stash32: Uint8Array,
    nonce: bigint,
    chainKey: bigint,
    amount: bigint,
    evmRecipient20: Uint8Array,
): Uint8Array {
    const prefix = Buffer.from('AttestCoin:claim:v1:', 'utf8');
    return new Uint8Array(
        Buffer.concat([
            prefix,
            Buffer.from(stash32),
            toLeU64(nonce),
            toLeU64(chainKey),
            toLeU128(amount),
            Buffer.from(evmRecipient20),
        ]),
    );
}

async function signSendInBlock(signer: KeyringPair, tx: SubmittableExtrinsic<'promise'>) {
    await new Promise<void>((resolve, reject) => {
        let settled = false;
        tx.signAndSend(signer, (r) => {
            if (settled || !r.status.isInBlock) {
                return;
            }
            settled = true;
            if (r.dispatchError) {
                reject(r.dispatchError);
            } else {
                resolve();
            }
        }).catch(reject);
    });
}

async function dispatchRootCall(api: ApiPromise, root: KeyringPair, inner: SubmittableExtrinsic<'promise'>) {
    const wrapped = (api.tx as any).sudo.sudo(inner);
    await signSendInBlock(root, wrapped);
}

describe('Precompile: attest-coin rewards (accrued / claim)', (): void => {
    let api: ApiPromise;
    let creditcoinWs: string;
    /** Creditcoin EVM — precompile + treasury token live here. */
    let creditcoinEvm: JsonRpcProvider;
    let anvilEvm: JsonRpcProvider;
    let evmWalletCc3: ethers.Wallet;
    let root: KeyringPair;
    let alice: KeyringPair;
    /** Mock ERC-20 on **Creditcoin** EVM (same bytecode as on Anvil); used for `setAttestCoinToken`, mint, balances, `claim`. */
    let tokenAddressCc3: string;

    beforeAll(async () => {
        await cryptoWaitReady();
        creditcoinWs = (global as any).CREDITCOIN_API_URL as string;
        const cc3Http = creditcoinHttpUrl();
        const anvilHttp = anvilHttpUrl();
        dbg('CREDITCOIN_API_URL (Substrate ws)=', creditcoinWs);
        dbg('CREDITCOIN_HTTP_URL (EVM)=', cc3Http);
        dbg('ANVIL1_HTTP_URL (deploy mock token)=', anvilHttp);

        ({ api } = await newApi(creditcoinWs));
        creditcoinEvm = new JsonRpcProvider(cc3Http);
        anvilEvm = new JsonRpcProvider(anvilHttp);

        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        const pk = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        evmWalletCc3 = new ethers.Wallet(pk, creditcoinEvm);
        const evmWalletAnvil = new ethers.Wallet(ANVIL_DEFAULT_ACCOUNT_0_PRIVATE_KEY, anvilEvm);

        // `integration-test-blockchain` runs zombienet attestors with //Alice-funded setup; Alice’s stash is already
        // on `attestation::Ledger` — no `registerAttestor` here.

        dbg('deploy MockAttestToken on Anvil (supported-chain local EVM)');
        const factoryAnvil = new ContractFactory(tokenArtifact.abi, tokenArtifact.bytecode, evmWalletAnvil);
        const deployedAnvil = await factoryAnvil.deploy();
        await deployedAnvil.waitForDeployment();
        dbg('Anvil token', await deployedAnvil.getAddress());

        dbg('deploy same mock on Creditcoin EVM for precompile treasury + runtime `setAttestCoinToken`');
        const factoryCc3 = new ContractFactory(tokenArtifact.abi, tokenArtifact.bytecode, evmWalletCc3);
        const deployedCc3 = await factoryCc3.deploy();
        await deployedCc3.waitForDeployment();
        tokenAddressCc3 = await deployedCc3.getAddress();

        const tokenCc3 = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, evmWalletCc3);
        const mintTx = await tokenCc3.mint(ATTEST_COIN_PRECOMPILE, ethers.parseEther('1000000'));
        await mintTx.wait();

        // Sequential sudo txs must be awaited; parallel `signAndSend` from the same account races the mempool
        // (1014: Priority is too low … replace another transaction already in the pool).
        await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddressCc3));
        await forElapsedBlocks(api, { minBlocks: 1 });
        await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.forceSettle());

        // Substrate `Accrued` / `ClaimNonce` persist on a long-lived dev node; this run’s ERC-20 is newly deployed with a
        // fixed mint. Top up the precompile so `transfer` can cover **all** current accrued points (not just this epoch).
        const preRead = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, creditcoinEvm);
        const stashRaw = decodeAddress(alice.address);
        const stashB32 = zeroPadValue(hexlify(stashRaw), 32);
        const accruedPts = await preRead.accrued(stashB32);
        let treasuryBal = await tokenCc3.balanceOf(ATTEST_COIN_PRECOMPILE);
        if (treasuryBal < accruedPts) {
            const topUp = await tokenCc3.mint(ATTEST_COIN_PRECOMPILE, accruedPts - treasuryBal);
            await topUp.wait();
            treasuryBal = await tokenCc3.balanceOf(ATTEST_COIN_PRECOMPILE);
        }
        dbg('treasury vs accrued', { treasuryBal: treasuryBal.toString(), accruedPts: accruedPts.toString() });
    }, 180_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('accrued(bytes32) returns non-zero after forceSettle', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, creditcoinEvm);
        const raw = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(raw), 32);
        const pts = await precompile.accrued(b32);
        expect(pts > 0n).toBe(true);
    });

    test('claim transfers MockAttestToken from precompile treasury with sr25519', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);

        const ptsBefore = await precompile.accrued(b32);
        const claimAmt = ptsBefore / 2n > 0n ? ptsBefore / 2n : ptsBefore;

        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, creditcoinEvm);
        const balBefore = await token.balanceOf(evmWalletCc3.address);

        const claimNonceBn = BigInt(
            ((await (api.query as any).attestCoinRewards.claimNonce(alice.address)) as { toString: () => string }).toString(),
        );

        const evmRecipient = ethers.getBytes(evmWalletCc3.address);
        const msg = buildClaimSigningMessage(stashU8, claimNonceBn, BigInt(chain_Anvil1_Key), claimAmt, evmRecipient);
        const sig = alice.sign(msg);
        if (sig.length !== 64) {
            throw new Error(`expected 64-byte sr25519 signature, got ${sig.length}`);
        }
        const sigHi = ethers.hexlify(sig.subarray(0, 32));
        const sigLo = ethers.hexlify(sig.subarray(32, 64));

        const tx = await precompile.claim(
            b32,
            claimNonceBn,
            BigInt(chain_Anvil1_Key),
            claimAmt,
            evmWalletCc3.address,
            sigHi,
            sigLo,
            { gasLimit: 1_000_000n },
        );
        await tx.wait();

        const balAfter = await token.balanceOf(evmWalletCc3.address);
        expect(balAfter - balBefore).toEqual(claimAmt);
    }, 120_000);

    test('deposit bridges ERC-20 into pallet-assets (mint to target creditcoin native account)', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, evmWalletCc3);

        const depositAmt = parseEther('3');
        await (await token.mint(evmWalletCc3.address, depositAmt)).wait();
        await (await token.approve(ATTEST_COIN_PRECOMPILE, depositAmt)).wait();

        const substrateBeneficiary = substrateAccountIdFromEvmAddress(evmWalletCc3.address);
        const assetId = 1;

        const acctBefore = await (api.query as any).assets.account(assetId, substrateBeneficiary);
        const bal0 = acctBefore.isSome ? BigInt(acctBefore.unwrap().balance.toString()) : 0n;

        const beneficiaryB32 = accountIdToBytes32(substrateBeneficiary);
        const tx = await precompile.depositTo(depositAmt, beneficiaryB32, { gasLimit: DEPOSIT_PRECOMPILE_GAS });
        await tx.wait();

        const acctAfter = await (api.query as any).assets.account(assetId, substrateBeneficiary);
        expect(acctAfter.isSome).toBe(true);
        const bal1 = BigInt(acctAfter.unwrap().balance.toString());
        expect(bal1 - bal0).toEqual(depositAmt);
    }, 120_000);

    /**
     * End-to-end: set a non-zero min bond, `depositTo` ERC-20 straight to the sr25519 stash, then
     * `registerAttestor` (no `force_transfer` — user-controlled beneficiary).
     */
    test('deposit attest coin, move to sr25519 stash, then registerAttestor', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, evmWalletCc3);
        const assetId = 1;

        const minBondWei = parseEther('100');
        await dispatchRootCall(
            api,
            root,
            (api.tx as any).attestation.setMinBondRequirement(chain_Anvil1_Key, minBondWei.toString()),
        );
        await forElapsedBlocks(api, { minBlocks: 1 });

        const minBond = BigInt(((await api.query.attestation.minBondRequirement(chain_Anvil1_Key)) as any).toString());
        expect(minBond).toEqual(minBondWei);

        const depositAmt = minBond * 2n;

        const kr = new Keyring({ type: 'sr25519', ss58Format: api.registry.chainSS58 });
        const stash = kr.addFromMnemonic(mnemonicGenerate(12));
        const attestor = kr.addFromMnemonic(mnemonicGenerate(12));

        const nativeFund = new BN('1000000000000000000000000');
        await dispatchRootCall(api, root, (api.tx as any).balances.forceSetBalance(stash.address, nativeFund));
        await forElapsedBlocks(api, { minBlocks: 1 });

        await (await token.mint(evmWalletCc3.address, depositAmt)).wait();
        await (await token.approve(ATTEST_COIN_PRECOMPILE, depositAmt)).wait();

        const stashB32 = accountIdToBytes32(stash.address);
        await (await precompile.depositTo(depositAmt, stashB32, { gasLimit: DEPOSIT_PRECOMPILE_GAS })).wait();
        await forElapsedBlocks(api, { minBlocks: 1 });

        const stashAcct = await (api.query as any).assets.account(assetId, stash.address);
        expect(stashAcct.isSome).toBe(true);
        const stashFree = BigInt(stashAcct.unwrap().balance.toString());
        expect(stashFree >= minBond).toBe(true);

        await signSendInBlock(
            stash,
            api.tx.attestation.registerAttestor(chain_Anvil1_Key, attestor.address),
        );
        await forElapsedBlocks(api, { minBlocks: 1 });

        const reg = await api.query.attestation.attestors(chain_Anvil1_Key, attestor.address);
        expect((reg as any).isSome).toBe(true);
    }, 180_000);
});
