import { Keyring } from '@polkadot/keyring';
import { BN, u8aConcat, u8aToHex, stringToU8a } from '@polkadot/util';
import { blake2AsU8a, cryptoWaitReady, decodeAddress, mnemonicGenerate } from '@polkadot/util-crypto';
import { ethers, hexlify, JsonRpcProvider, parseEther, WebSocketProvider, zeroPadValue, ContractFactory } from 'ethers';

import { newApi, ApiPromise, KeyringPair, MICROUNITS_PER_CTC } from '../../../lib';
import { chain_Anvil1_Key } from '../pallets/supported-chains/consts';
import type { SubmittableExtrinsic } from '@polkadot/api/types';
import { fundFromSudo } from '../../integration-tests/helpers';

// eslint-disable-next-line @typescript-eslint/no-require-imports
import tokenArtifact = require('../artifacts/MockAttestToken.json');
// eslint-disable-next-line @typescript-eslint/no-require-imports
import feeOnTransferTokenArtifact = require('../artifacts/FeeOnTransferAttestToken.json');
// eslint-disable-next-line @typescript-eslint/no-require-imports
import precompileAbi = require('../artifacts/attest_coin.json');
import { forElapsedBlocks } from '../../utils';
import { evmAddressToSubstrateAccountId } from '../../../lib/evm/address';

const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';

const ATTEST_COIN_ASSET_ID = 1;

/** Headroom for ERC-20 subcall + precompile `mint` dispatch (runtime avoids double-counting PoV vs `try_dispatch`). */
const DEPOSIT_PRECOMPILE_GAS = 8_000_000;

/** Headroom for `pallet-assets` `burn` dispatch + ERC-20 `transfer` subcall (+ rollback mint on failure). */
const WITHDRAW_PRECOMPILE_GAS = 8_000_000;

/** `claim` includes sr25519 verify + ERC-20 transfer; use a numeric limit (ethers ignores bigint overrides on some paths). */
const CLAIM_PRECOMPILE_GAS = 3_000_000;

// substrateAccountIdFromEvmAddress is exported from cli/src/lib/evm/address.ts as evmAddressToSubstrateAccountId

/** `bytes32` for precompile `depositTo` / Solidity - 32-byte raw Substrate `AccountId`. */
function accountIdToBytes32(accountSs58OrRaw: string | Uint8Array): string {
    const raw = typeof accountSs58OrRaw === 'string' ? decodeAddress(accountSs58OrRaw) : accountSs58OrRaw;
    return zeroPadValue(hexlify(raw), 32);
}

/**
 * Foundry Anvil account #0 - always pre-funded. Same key as `attestor/scripts/Transfer.js` (`getSigner`).
 * Alith / `CREDITCOIN_EVM_PRIVATE_KEY('alice')` is **not** funded on vanilla Anvil.
 */
const ANVIL_DEFAULT_ACCOUNT_0_PRIVATE_KEY = '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';

/** Foundry Anvil / local EVM - default matches CI `anvil --port 8141`; override with `ANVIL1_HTTP_URL` (e.g. `http://127.0.0.1:8545`). */
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

/** `chain_key` in claim preimage - Anvil1 matches CI zombienet `--chain-key=2`. */
function toLeU64(n: bigint): Buffer {
    let v = n;
    const b = Buffer.alloc(8);
    for (let i = 0; i < 8; i++) {
        b[i] = Number(v % 256n);
        v /= 256n;
    }
    return b;
}

function toLeU128(n: bigint): Buffer {
    let v = n;
    const b = Buffer.alloc(16);
    for (let i = 0; i < 16; i++) {
        b[i] = Number(v % 256n);
        v /= 256n;
    }
    return b;
}

/** Must match `pallet_attest_coin_rewards::Pallet::claim_signing_message`. */
function buildClaimSigningMessage(
    genesisHash32: Uint8Array,
    stash32: Uint8Array,
    nonce: bigint,
    chainKey: bigint,
    amount: bigint,
    evmRecipient20: Uint8Array,
): Uint8Array {
    const prefix = Buffer.from('AttestCoin:claim:v2:', 'utf8');
    return new Uint8Array(
        Buffer.concat([
            prefix,
            Buffer.from(genesisHash32),
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
                reject(new Error(r.dispatchError.toString()));
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

/** Matches `AttestationBondPoolAccount` (`PalletId(*b"att/bond")`). */
function attestationBondPoolAccountId(): Uint8Array {
    return blake2AsU8a(u8aConcat(stringToU8a('modl'), stringToU8a('att/bond')));
}

/** ERC-20 backing required for non-pool pallet-assets attest-coin (see precompile treasury guard). */
async function getWithdrawableBacking(api: ApiPromise): Promise<bigint> {
    const assetOpt = await (api.query as any).assets.asset(ATTEST_COIN_ASSET_ID);
    if (assetOpt.isNone) {
        return 0n;
    }
    const supply = BigInt(assetOpt.unwrap().supply.toString());
    const poolAcct = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, attestationBondPoolAccountId());
    const poolBal = poolAcct.isSome ? BigInt(poolAcct.unwrap().balance.toString()) : 0n;
    return supply > poolBal ? supply - poolBal : 0n;
}

async function ensureTreasuryBalance(token: ethers.Contract, minBalance: bigint): Promise<void> {
    const bal: bigint = await token.balanceOf(ATTEST_COIN_PRECOMPILE);
    if (bal < minBalance) {
        await (await token.mint(ATTEST_COIN_PRECOMPILE, minBalance - bal)).wait();
    }
}

function leU64Hex(n: bigint): string {
    return hexlify(toLeU64(n));
}

async function clearAttestCoinToken(api: ApiPromise, root: KeyringPair): Promise<void> {
    const key = (api.query as any).attestCoinRewards.attestCoinErc20.key();
    await dispatchRootCall(api, root, (api.tx as any).system.killStorage([key]));
    await forElapsedBlocks(api, { minBlocks: 1 });
}

async function setClaimNonce(api: ApiPromise, root: KeyringPair, stash: string, nonce: bigint): Promise<void> {
    const key = (api.query as any).attestCoinRewards.claimNonce.key(stash);
    await dispatchRootCall(api, root, (api.tx as any).system.setStorage([[key, leU64Hex(nonce)]]));
    await forElapsedBlocks(api, { minBlocks: 1 });
}

async function precompileTxOverrides(provider: WebSocketProvider, gasLimit: number) {
    const latestBlock = await provider.getBlock('latest');
    const baseFeePerGas = latestBlock?.baseFeePerGas ?? 1_000_000_000n;
    const feeData = await provider.getFeeData();
    const maxPriorityFeePerGas = feeData.maxPriorityFeePerGas ?? 1_000_000_000n;
    const maxFeePerGas = baseFeePerGas * 10n + maxPriorityFeePerGas;
    return { gasLimit, maxFeePerGas, maxPriorityFeePerGas };
}

/** Non-sufficient pallet-assets accounts need a native-balance provider on the beneficiary. */
async function ensureNativeProvider(api: ApiPromise, root: KeyringPair, accountId: Uint8Array | string): Promise<void> {
    const nativeTopUp = new BN('1000000000000000000000000');
    const accountIdVariant = 'Id';
    const who = typeof accountId === 'string' ? accountId : { [accountIdVariant]: u8aToHex(accountId) };
    await dispatchRootCall(api, root, (api.tx as any).balances.forceSetBalance(who, nativeTopUp.toString()));
    await forElapsedBlocks(api, { minBlocks: 1 });
}

/** Genesis + migration must set issuer/admin to the precompile; deposit/withdraw rely on these roles. */
async function expectAttestCoinAssetRoles(api: ApiPromise): Promise<void> {
    const precompileAcct = evmAddressToSubstrateAccountId(ATTEST_COIN_PRECOMPILE);
    const assetOpt = await (api.query as any).assets.asset(ATTEST_COIN_ASSET_ID);
    if (assetOpt.isNone) {
        throw new Error(`pallet-assets id ${ATTEST_COIN_ASSET_ID} missing (genesis/migration)`);
    }
    const details = assetOpt.unwrap();
    const precompileHex = u8aToHex(precompileAcct).toLowerCase();
    expect(details.issuer.toHex().toLowerCase()).toEqual(precompileHex);
    expect(details.admin.toHex().toLowerCase()).toEqual(precompileHex);
}

describe('Precompile: attest-coin rewards (accrued / claim)', (): void => {
    let api: ApiPromise;
    let creditcoinWs: string;
    /** Creditcoin EVM - precompile + treasury token live here. */
    let creditcoinEvm: WebSocketProvider;
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
        creditcoinEvm = new WebSocketProvider(creditcoinWs);
        anvilEvm = new JsonRpcProvider(anvilHttp);

        root = (global as any).CREDITCOIN_CREATE_SIGNER('sudo');
        alice = (global as any).CREDITCOIN_CREATE_SIGNER('alice');

        const pk = (global as any).CREDITCOIN_EVM_PRIVATE_KEY('alice');
        evmWalletCc3 = new ethers.Wallet(pk, creditcoinEvm);
        const evmWalletAnvil = new ethers.Wallet(ANVIL_DEFAULT_ACCOUNT_0_PRIVATE_KEY, anvilEvm);

        // `integration-test-blockchain` runs zombienet attestors with //Alice-funded setup; Alice's stash is already
        // on `attestation::Ledger` - no `registerAttestor` here.

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

        await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddressCc3));
        await forElapsedBlocks(api, { minBlocks: 1 });

        await expectAttestCoinAssetRoles(api);

        const fundResult = await fundFromSudo(api, evmWalletCc3.address, MICROUNITS_PER_CTC.mul(new BN(2_000_000)));
        expect(fundResult.status).toBe(0);

        // `pallet-assets` balances for non-sufficient assets require the holder to have a native-balance
        // `provider`. Alice's EVM-mapped Substrate account is otherwise empty, so `deposit`/`depositTo`
        // minting into that account reverts with `CannotCreate` / dispatch failure.
        await ensureNativeProvider(api, root, evmAddressToSubstrateAccountId(evmWalletCc3.address));

        // Substrate `Accrued` / `ClaimNonce` persist on a long-lived dev node; this run's ERC-20 is newly deployed with a
        // fixed mint. Claims share the treasury with deposit-backed withdraws, so fund accrued rewards **and**
        // withdrawable pallet-assets supply (total supply minus bond-pool balance).
        const preRead = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, creditcoinEvm);
        const stashRaw = decodeAddress(alice.address);
        const stashB32 = zeroPadValue(hexlify(stashRaw), 32);
        const accruedPts = await preRead.accrued(stashB32);
        const withdrawableBacking = await getWithdrawableBacking(api);
        const treasuryNeeded = accruedPts + withdrawableBacking;
        await ensureTreasuryBalance(tokenCc3, treasuryNeeded);
        const treasuryBal = await tokenCc3.balanceOf(ATTEST_COIN_PRECOMPILE);
        dbg('treasury vs accrued/backing', {
            treasuryBal: treasuryBal.toString(),
            accruedPts: accruedPts.toString(),
            withdrawableBacking: withdrawableBacking.toString(),
        });
    }, 180_000);

    afterAll(async () => {
        await api.disconnect();
    });

    test('accrued(bytes32) returns a non-negative value', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, creditcoinEvm);
        const raw = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(raw), 32);
        const pts = await precompile.accrued(b32);
        // Accrued may be 0 if attestors haven't completed a full epoch yet.
        expect(pts >= 0n).toBe(true);
    });

    test('claim transfers MockAttestToken from precompile treasury with sr25519', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);

        const ptsBefore = await precompile.accrued(b32);
        // Skip claim test if accrued is 0 (attestors haven't completed a full epoch).
        if (ptsBefore === 0n) {
            return;
        }
        const claimAmt = ptsBefore / 2n > 0n ? ptsBefore / 2n : ptsBefore;

        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, creditcoinEvm);
        const withdrawableBacking = await getWithdrawableBacking(api);
        await ensureTreasuryBalance(token, claimAmt + withdrawableBacking);

        const balBefore = await token.balanceOf(evmWalletCc3.address);

        const claimNonceBn = BigInt(
            (
                (await (api.query as any).attestCoinRewards.claimNonce(alice.address)) as { toString: () => string }
            ).toString(),
        );

        const evmRecipient = ethers.getBytes(evmWalletCc3.address);
        const genesisHashHex = await api.rpc.chain.getBlockHash(0);
        const genesisHashU8 = ethers.getBytes(genesisHashHex);
        const msg = buildClaimSigningMessage(
            genesisHashU8,
            stashU8,
            claimNonceBn,
            BigInt(chain_Anvil1_Key),
            claimAmt,
            evmRecipient,
        );
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
            await precompileTxOverrides(creditcoinEvm, CLAIM_PRECOMPILE_GAS),
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

        const substrateBeneficiary = evmAddressToSubstrateAccountId(evmWalletCc3.address);
        await ensureNativeProvider(api, root, substrateBeneficiary);

        const acctBefore = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, substrateBeneficiary);
        const bal0 = acctBefore.isSome ? BigInt(acctBefore.unwrap().balance.toString()) : 0n;

        const beneficiaryB32 = accountIdToBytes32(substrateBeneficiary);
        const tx = await precompile.depositTo(
            depositAmt,
            beneficiaryB32,
            await precompileTxOverrides(creditcoinEvm, DEPOSIT_PRECOMPILE_GAS),
        );
        await tx.wait();

        const acctAfter = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, substrateBeneficiary);
        expect(acctAfter.isSome).toBe(true);
        const bal1 = BigInt(acctAfter.unwrap().balance.toString());
        expect(bal1 - bal0).toEqual(depositAmt);
    }, 120_000);

    test('withdraw burns pallet-assets attest coin and sends ERC-20 back to caller', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, evmWalletCc3);
        const mapped = evmAddressToSubstrateAccountId(evmWalletCc3.address);
        await ensureNativeProvider(api, root, mapped);

        // First deposit some attest coin so the EVM caller's mapped substrate account has a balance
        // that we can subsequently withdraw back out as ERC-20.
        const amount = parseEther('5');
        await (await token.mint(evmWalletCc3.address, amount)).wait();
        await (await token.approve(ATTEST_COIN_PRECOMPILE, amount)).wait();

        const acctBefore = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
        const palletBalBefore = acctBefore.isSome ? BigInt(acctBefore.unwrap().balance.toString()) : 0n;

        await (
            await precompile.deposit(amount, await precompileTxOverrides(creditcoinEvm, DEPOSIT_PRECOMPILE_GAS))
        ).wait();

        const acctAfterDeposit = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
        expect(acctAfterDeposit.isSome).toBe(true);
        const palletBalAfterDeposit = BigInt(acctAfterDeposit.unwrap().balance.toString());
        expect(palletBalAfterDeposit - palletBalBefore).toEqual(amount);

        // Ensure the precompile treasury can cover the withdraw `transfer`.
        const treasuryBal: bigint = await token.balanceOf(ATTEST_COIN_PRECOMPILE);
        if (treasuryBal < amount) {
            await (await token.mint(ATTEST_COIN_PRECOMPILE, amount - treasuryBal)).wait();
        }

        const erc20Before: bigint = await token.balanceOf(evmWalletCc3.address);

        const tx = await precompile.withdraw(
            amount,
            await precompileTxOverrides(creditcoinEvm, WITHDRAW_PRECOMPILE_GAS),
        );
        const receipt = await tx.wait();
        expect(receipt?.status).toEqual(1);

        // Substrate balance decreased by exactly `amount`.
        const acctAfterWithdraw = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
        const palletBalAfterWithdraw = acctAfterWithdraw.isSome
            ? BigInt(acctAfterWithdraw.unwrap().balance.toString())
            : 0n;
        expect(palletBalAfterDeposit - palletBalAfterWithdraw).toEqual(amount);

        // ERC-20 balance increased by exactly `amount`.
        const erc20After: bigint = await token.balanceOf(evmWalletCc3.address);
        expect(erc20After - erc20Before).toEqual(amount);
    }, 180_000);

    test('withdraw reverts with zero amount', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        // The precompile returns the raw bytes `"zero amount"` (no Solidity `Error(string)` selector),
        // so ethers cannot decode a reason - match the hex payload (`hex("zero amount")`) directly.
        await expect(precompile.withdraw.staticCall(0n, { gasLimit: WITHDRAW_PRECOMPILE_GAS })).rejects.toThrow(
            /0x7a65726f20616d6f756e74/,
        );
    }, 60_000);

    test('withdraw reverts when caller has insufficient pallet-assets balance', async () => {
        // Use a fresh EVM wallet whose mapped substrate account has never held any attest coin.
        const freshWallet = ethers.Wallet.createRandom().connect(creditcoinEvm);

        // Fund native EVM balance so the fresh wallet can pay gas.
        const fundTx = await evmWalletCc3.sendTransaction({
            to: freshWallet.address,
            value: parseEther('100'),
        });
        await fundTx.wait();

        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, freshWallet);

        // `pallet_assets::burn` on an account with no balance dispatches an error which surfaces as
        // an EVM revert from the precompile.
        await expect(
            precompile.withdraw.staticCall(parseEther('1'), { gasLimit: WITHDRAW_PRECOMPILE_GAS }),
        ).rejects.toThrow();
    }, 120_000);

    test('claim reverts with bad sr25519 signature', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);
        const claimNonceBn = BigInt(
            (
                (await (api.query as any).attestCoinRewards.claimNonce(alice.address)) as { toString: () => string }
            ).toString(),
        );
        const sigHi = ethers.hexlify(new Uint8Array(32));
        const sigLo = ethers.hexlify(new Uint8Array(32));

        await expect(
            precompile.claim.staticCall(
                b32,
                claimNonceBn,
                BigInt(chain_Anvil1_Key),
                1n,
                evmWalletCc3.address,
                sigHi,
                sigLo,
                { gasLimit: CLAIM_PRECOMPILE_GAS },
            ),
        ).rejects.toThrow(/626164207369676e6174757265/);
    }, 60_000);

    test('claim reverts with bad nonce', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);
        const currentNonce = BigInt(
            (
                (await (api.query as any).attestCoinRewards.claimNonce(alice.address)) as { toString: () => string }
            ).toString(),
        );
        const badNonce = currentNonce + 1n;
        const msg = buildClaimSigningMessage(
            ethers.getBytes(await api.rpc.chain.getBlockHash(0)),
            stashU8,
            badNonce,
            BigInt(chain_Anvil1_Key),
            1n,
            ethers.getBytes(evmWalletCc3.address),
        );
        const sig = alice.sign(msg);

        await expect(
            precompile.claim.staticCall(
                b32,
                badNonce,
                BigInt(chain_Anvil1_Key),
                1n,
                evmWalletCc3.address,
                ethers.hexlify(sig.subarray(0, 32)),
                ethers.hexlify(sig.subarray(32, 64)),
                { gasLimit: CLAIM_PRECOMPILE_GAS },
            ),
        ).rejects.toThrow(/626164206e6f6e6365/);
    }, 60_000);

    test('deposit reverts when token is not configured', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        try {
            await clearAttestCoinToken(api, root);

            await expect(
                precompile.deposit.staticCall(parseEther('1'), { gasLimit: DEPOSIT_PRECOMPILE_GAS }),
            ).rejects.toThrow(/746f6b656e206e6f7420636f6e66696775726564/);
        } finally {
            await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddressCc3));
            await forElapsedBlocks(api, { minBlocks: 1 });
        }
    }, 120_000);

    test('deposit rejects fee-on-transfer ERC-20 and rolls back token transfer', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const factory = new ContractFactory(
            feeOnTransferTokenArtifact.abi,
            feeOnTransferTokenArtifact.bytecode,
            evmWalletCc3,
        );
        const deployed = await factory.deploy();
        await deployed.waitForDeployment();
        const tokenAddress = await deployed.getAddress();
        const token = new ethers.Contract(tokenAddress, feeOnTransferTokenArtifact.abi, evmWalletCc3);
        const amount = parseEther('2');
        const mapped = evmAddressToSubstrateAccountId(evmWalletCc3.address);

        try {
            await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddress));
            await forElapsedBlocks(api, { minBlocks: 1 });
            await ensureNativeProvider(api, root, mapped);
            await (await token.mint(evmWalletCc3.address, amount)).wait();
            await (await token.approve(ATTEST_COIN_PRECOMPILE, amount)).wait();

            const callerBefore = await token.balanceOf(evmWalletCc3.address);
            const treasuryBefore = await token.balanceOf(ATTEST_COIN_PRECOMPILE);
            const acctBefore = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
            const palletBefore = acctBefore.isSome ? BigInt(acctBefore.unwrap().balance.toString()) : 0n;

            await expect(precompile.deposit.staticCall(amount, { gasLimit: DEPOSIT_PRECOMPILE_GAS })).rejects.toThrow(
                /6e6f6e2d7374616e6461726420746f6b656e/,
            );
            const tx = await precompile.deposit(
                amount,
                await precompileTxOverrides(creditcoinEvm, DEPOSIT_PRECOMPILE_GAS),
            );
            await expect(tx.wait()).rejects.toThrow();

            expect(await token.balanceOf(evmWalletCc3.address)).toEqual(callerBefore);
            expect(await token.balanceOf(ATTEST_COIN_PRECOMPILE)).toEqual(treasuryBefore);
            const acctAfter = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
            const palletAfter = acctAfter.isSome ? BigInt(acctAfter.unwrap().balance.toString()) : 0n;
            expect(palletAfter).toEqual(palletBefore);
        } finally {
            await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddressCc3));
            await forElapsedBlocks(api, { minBlocks: 1 });
        }
    }, 180_000);

    test('claim rejects max nonce replay without mutating state', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, creditcoinEvm);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);
        const previousNonce = BigInt(
            (
                (await (api.query as any).attestCoinRewards.claimNonce(alice.address)) as { toString: () => string }
            ).toString(),
        );
        const accruedBefore = await precompile.accrued(b32);
        const balanceBefore = await token.balanceOf(evmWalletCc3.address);
        const maxNonce = 2n ** 64n - 1n;
        const msg = buildClaimSigningMessage(
            ethers.getBytes(await api.rpc.chain.getBlockHash(0)),
            stashU8,
            maxNonce,
            BigInt(chain_Anvil1_Key),
            1n,
            ethers.getBytes(evmWalletCc3.address),
        );
        const sig = alice.sign(msg);

        try {
            await setClaimNonce(api, root, alice.address, maxNonce);

            await expect(
                precompile.claim.staticCall(
                    b32,
                    maxNonce,
                    BigInt(chain_Anvil1_Key),
                    1n,
                    evmWalletCc3.address,
                    ethers.hexlify(sig.subarray(0, 32)),
                    ethers.hexlify(sig.subarray(32, 64)),
                    { gasLimit: CLAIM_PRECOMPILE_GAS },
                ),
            ).rejects.toThrow(/626164206e6f6e6365/);
            expect(await precompile.accrued(b32)).toEqual(accruedBefore);
            expect(await token.balanceOf(evmWalletCc3.address)).toEqual(balanceBefore);
        } finally {
            await setClaimNonce(api, root, alice.address, previousNonce);
        }
    }, 120_000);

    test('claim reverts when treasury would impair deposit backing', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const stashU8 = decodeAddress(alice.address);
        const b32 = zeroPadValue(hexlify(stashU8), 32);

        const ptsBefore = await precompile.accrued(b32);
        if (ptsBefore === 0n) {
            return;
        }

        const factory = new ContractFactory(tokenArtifact.abi, tokenArtifact.bytecode, evmWalletCc3);
        const isolated = await factory.deploy();
        await isolated.waitForDeployment();
        const isolatedAddr = await isolated.getAddress();
        const token = new ethers.Contract(isolatedAddr, tokenArtifact.abi, evmWalletCc3);

        await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(isolatedAddr));
        await forElapsedBlocks(api, { minBlocks: 1 });

        const depositAmt = parseEther('10');
        await (await token.mint(evmWalletCc3.address, depositAmt)).wait();
        await (await token.approve(ATTEST_COIN_PRECOMPILE, depositAmt)).wait();
        await ensureNativeProvider(api, root, evmAddressToSubstrateAccountId(evmWalletCc3.address));
        await (
            await precompile.deposit(depositAmt, await precompileTxOverrides(creditcoinEvm, DEPOSIT_PRECOMPILE_GAS))
        ).wait();

        const claimAmt = ptsBefore / 2n > 0n ? ptsBefore / 2n : ptsBefore;
        const claimNonceBn = BigInt(
            (
                (await (api.query as any).attestCoinRewards.claimNonce(alice.address)) as { toString: () => string }
            ).toString(),
        );

        const genesisHashHex = await api.rpc.chain.getBlockHash(0);
        const msg = buildClaimSigningMessage(
            ethers.getBytes(genesisHashHex),
            stashU8,
            claimNonceBn,
            BigInt(chain_Anvil1_Key),
            claimAmt,
            ethers.getBytes(evmWalletCc3.address),
        );
        const sig = alice.sign(msg);
        const sigHi = ethers.hexlify(sig.subarray(0, 32));
        const sigLo = ethers.hexlify(sig.subarray(32, 64));

        await expect(
            precompile.claim.staticCall(
                b32,
                claimNonceBn,
                BigInt(chain_Anvil1_Key),
                claimAmt,
                evmWalletCc3.address,
                sigHi,
                sigLo,
                { gasLimit: CLAIM_PRECOMPILE_GAS },
            ),
        ).rejects.toThrow(/636c61696d20776f756c6420696d70616972206465706f736974206261636b696e67/);

        await dispatchRootCall(api, root, (api.tx as any).attestCoinRewards.setAttestCoinToken(tokenAddressCc3));
        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 180_000);

    test('withdraw rolls back ERC-20 transfer when burn fails', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, evmWalletCc3);
        const precompileSubstrate = evmAddressToSubstrateAccountId(ATTEST_COIN_PRECOMPILE);
        const amount = parseEther('2');

        await (await token.mint(evmWalletCc3.address, amount)).wait();
        await (await token.approve(ATTEST_COIN_PRECOMPILE, amount)).wait();
        const mapped = evmAddressToSubstrateAccountId(evmWalletCc3.address);
        await ensureNativeProvider(api, root, mapped);

        await (
            await precompile.deposit(amount, await precompileTxOverrides(creditcoinEvm, DEPOSIT_PRECOMPILE_GAS))
        ).wait();

        const erc20Before = await token.balanceOf(evmWalletCc3.address);
        const treasuryBefore = await token.balanceOf(ATTEST_COIN_PRECOMPILE);
        const acct = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
        const palletBefore = BigInt(acct.unwrap().balance.toString());

        const assetDetails = (await (api.query as any).assets.asset(ATTEST_COIN_ASSET_ID)).unwrap();
        const isFrozen = assetDetails.status.type === 'Frozen';
        await dispatchRootCall(
            api,
            root,
            (api.tx as any).assets.forceAssetStatus(
                ATTEST_COIN_ASSET_ID,
                precompileSubstrate,
                precompileSubstrate,
                alice.address,
                root.address,
                assetDetails.minBalance,
                assetDetails.isSufficient,
                isFrozen,
            ),
        );
        await forElapsedBlocks(api, { minBlocks: 1 });

        await expect(precompile.withdraw.staticCall(amount, { gasLimit: WITHDRAW_PRECOMPILE_GAS })).rejects.toThrow();

        expect(await token.balanceOf(evmWalletCc3.address)).toEqual(erc20Before);
        expect(await token.balanceOf(ATTEST_COIN_PRECOMPILE)).toEqual(treasuryBefore);
        const acctAfter = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, mapped);
        expect(BigInt(acctAfter.unwrap().balance.toString())).toEqual(palletBefore);

        await dispatchRootCall(
            api,
            root,
            (api.tx as any).assets.forceAssetStatus(
                ATTEST_COIN_ASSET_ID,
                precompileSubstrate,
                precompileSubstrate,
                precompileSubstrate,
                root.address,
                assetDetails.minBalance,
                assetDetails.isSufficient,
                isFrozen,
            ),
        );
        await forElapsedBlocks(api, { minBlocks: 1 });
    }, 180_000);

    /**
     * End-to-end: set a non-zero min bond, `depositTo` ERC-20 straight to the sr25519 stash, then
     * `registerAttestor` (no `force_transfer` - user-controlled beneficiary).
     */
    test('deposit attest coin, move to sr25519 stash, then registerAttestor', async () => {
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, evmWalletCc3);
        const token = new ethers.Contract(tokenAddressCc3, tokenArtifact.abi, evmWalletCc3);

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

        await ensureNativeProvider(api, root, decodeAddress(stash.address));

        await (await token.mint(evmWalletCc3.address, depositAmt)).wait();
        await (await token.approve(ATTEST_COIN_PRECOMPILE, depositAmt)).wait();

        const stashB32 = accountIdToBytes32(stash.address);
        await (
            await precompile.depositTo(
                depositAmt,
                stashB32,
                await precompileTxOverrides(creditcoinEvm, DEPOSIT_PRECOMPILE_GAS),
            )
        ).wait();
        await forElapsedBlocks(api, { minBlocks: 1 });

        const stashAcct = await (api.query as any).assets.account(ATTEST_COIN_ASSET_ID, stash.address);
        expect(stashAcct.isSome).toBe(true);
        const stashFree = BigInt(stashAcct.unwrap().balance.toString());
        expect(stashFree >= minBond).toBe(true);

        await signSendInBlock(stash, api.tx.attestation.registerAttestor(chain_Anvil1_Key, attestor.address));
        await forElapsedBlocks(api, { minBlocks: 1 });

        const reg = await api.query.attestation.attestors(chain_Anvil1_Key, attestor.address);
        expect((reg as any).isSome).toBe(true);
    }, 180_000);
});
