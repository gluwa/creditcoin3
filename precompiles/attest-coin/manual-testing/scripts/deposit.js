'use strict';

// Step 6 — bridge the stash's ERC-20 into pallet-assets attest coin.
//
// `deposit(amount)` pulls ERC-20 from the caller with `transferFrom` (which is
// why the approve below targets the precompile — it is the approved spender in
// its own subcall) and mints the same amount of asset id 1 to the caller's
// *mapped* Substrate account, `blake2_256("evm:" || address)`.
//
// That mapped account is the stash: the account `pallet-attestation` bonds from
// in step 7 and accrues rewards to. Nothing is bonded yet — this is liquid
// attest coin sitting in pallet-assets.
//
// Usage:
//   node scripts/deposit.js          # 100 ATC, matching the step 2 min bond
//   node scripts/deposit.js 250

const fs = require('fs');
const path = require('path');
const { ethers } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { blake2AsU8a, encodeAddress } = require('@polkadot/util-crypto');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const ARTIFACT_PATH = path.join(
    REPO_ROOT,
    'cli/src/test/blockchain-tests/artifacts/MockAttestToken.json',
);
const PRECOMPILE_ABI_PATH = path.join(
    REPO_ROOT,
    'cli/src/test/blockchain-tests/artifacts/attest_coin.json',
);

/** Attest-coin precompile, `PrecompileAt<AddressU64<4053>>` — 4053 == 0xfd5. */
const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';
/** `ATTEST_COIN_ASSET_ID` in runtime/src/lib.rs. */
const ATTEST_COIN_ASSET_ID = 1;
/** `SS58Prefix` in runtime/src/lib.rs. */
const SS58_PREFIX = 42;
/**
 * `deposit` runs two ERC-20 subcalls plus a `pallet_assets::mint` dispatch.
 * estimateGas cannot see through the Substrate dispatch, so the limit is fixed
 * — same value the blockchain integration tests use.
 */
const DEPOSIT_GAS = 8_000_000;

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const WS_URL = process.env.CC3_WS_URL || RPC_URL.replace(/^http/, 'ws');

/** Raw 32-byte AccountId for an EVM address (`HashedAddressMapping<BlakeTwo256>`). */
function mappedAccountId(evmAddress) {
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(ethers.getBytes(evmAddress), 4);
    return blake2AsU8a(payload, 256);
}

/** Liquid attest coin held by a Substrate account, in base units. */
async function assetBalance(api, ss58) {
    const account = await api.query.assets.account(ATTEST_COIN_ASSET_ID, ss58);
    return account.isSome ? BigInt(account.unwrap().balance.toString()) : 0n;
}

/** Bid well above the dev chain's base fee so the tx is not stuck behind it. */
async function feeOverrides(provider) {
    const latest = await provider.getBlock('latest');
    const baseFee = latest?.baseFeePerGas ?? 1_000_000_000n;
    const priority = (await provider.getFeeData()).maxPriorityFeePerGas ?? 1_000_000_000n;
    return { gasLimit: DEPOSIT_GAS, maxFeePerGas: baseFee * 10n + priority, maxPriorityFeePerGas: priority };
}

/**
 * Frontier surfaces a failed Substrate dispatch as an EVM revert whose reason
 * embeds the pallet error name. Dig it out so the failure is readable.
 */
function explainRevert(error) {
    const message = error instanceof Error ? error.message : String(error);
    const pallet = message.match(/message:\s*Some\(\\?"([A-Za-z0-9_]+)\\?"\)/);
    if (pallet) {
        return `reverted with pallet error ${pallet[1]}`;
    }
    const reason = message.match(/reason="((?:\\.|[^"\\])*)"/);
    return reason ? `reverted: ${reason[1]}` : message;
}

async function main() {
    const amountArg = process.argv[2] || '100';
    const amount = ethers.parseUnits(amountArg, 18);
    if (amount <= 0n) {
        throw new Error(`amount must be positive, got "${amountArg}"`);
    }

    const { STASH_PRIVATE_KEY, ATTESTCOIN_ERC20 } = process.env;
    if (!STASH_PRIVATE_KEY) {
        throw new Error('STASH_PRIVATE_KEY is not set — run scripts/new-stash.js (step 5.2)');
    }
    if (!ATTESTCOIN_ERC20) {
        throw new Error('ATTESTCOIN_ERC20 is not set — run scripts/deploy-erc20.js (step 3)');
    }

    const provider = new ethers.JsonRpcProvider(RPC_URL);
    const api = await ApiPromise.create({
        provider: new WsProvider(WS_URL, 1_000, {}, 20_000),
        noInitWarn: true,
        throwOnConnect: true,
    });

    try {
        const stash = new ethers.Wallet(STASH_PRIVATE_KEY, provider);
        const stashSs58 = encodeAddress(mappedAccountId(stash.address), SS58_PREFIX);
        const tokenArtifact = JSON.parse(fs.readFileSync(ARTIFACT_PATH, 'utf8'));
        const precompileAbi = JSON.parse(fs.readFileSync(PRECOMPILE_ABI_PATH, 'utf8'));
        const token = new ethers.Contract(ATTESTCOIN_ERC20, tokenArtifact.abi, stash);
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, precompileAbi, stash);

        // The precompile mints whatever `attestCoinRewards.attestCoinErc20` names,
        // not whatever is in .env. If they disagree the deposit would pull from
        // one token and back the mint with another.
        const configured = await api.query.attestCoinRewards.attestCoinErc20();
        if (configured.isNone) {
            throw new Error(
                'attestCoinRewards.attestCoinErc20 is unset — do step 4 (sudo setAttestCoinToken) first',
            );
        }
        const configuredAddress = ethers.getAddress(configured.unwrap().toHex());
        if (configuredAddress !== ethers.getAddress(ATTESTCOIN_ERC20)) {
            throw new Error(
                `the runtime is configured for ${configuredAddress} but .env has ${ATTESTCOIN_ERC20}\n` +
                    'redo step 4 with the current address, or re-deploy',
            );
        }

        const erc20Before = BigInt((await token.balanceOf(stash.address)).toString());
        const assetsBefore = await assetBalance(api, stashSs58);
        const ctc = BigInt((await api.query.system.account(stashSs58)).data.free.toString());

        console.log(`stash EVM      ${stash.address}`);
        console.log(`stash SS58     ${stashSs58}`);
        console.log(`token          ${configuredAddress}`);
        console.log(`depositing     ${ethers.formatUnits(amount, 18)} ATC`);
        console.log();
        console.log(`stash CTC      ${ethers.formatEther(ctc)}`);
        console.log(`stash ERC-20   ${ethers.formatUnits(erc20Before, 18)} ATC`);
        console.log(`stash assets   ${ethers.formatUnits(assetsBefore, 18)} ATC`);

        if (erc20Before < amount) {
            throw new Error(
                `stash holds ${ethers.formatUnits(erc20Before, 18)} ATC of ERC-20, needs ` +
                    `${ethers.formatUnits(amount, 18)} — run scripts/fund-erc20.js (step 5.2)`,
            );
        }
        // Attest coin is a non-sufficient asset, so the beneficiary needs a
        // native-balance provider before it can hold any. Without CTC the mint
        // fails inside the dispatch rather than at the ERC-20 layer.
        if (ctc === 0n) {
            throw new Error(
                `${stashSs58} has no CTC, so the mint would fail — do the step 5.2 CTC funding first`,
            );
        }

        console.log(`\napproving      ${ATTEST_COIN_PRECOMPILE}`);
        await (await token.approve(ATTEST_COIN_PRECOMPILE, amount)).wait();

        const receipt = await (await precompile.deposit(amount, await feeOverrides(provider))).wait();
        console.log(`deposited      tx ${receipt.hash}`);

        const erc20After = BigInt((await token.balanceOf(stash.address)).toString());
        const assetsAfter = await assetBalance(api, stashSs58);

        console.log();
        console.log(`stash ERC-20   ${ethers.formatUnits(erc20After, 18)} ATC  (-${ethers.formatUnits(erc20Before - erc20After, 18)})`);
        console.log(`stash assets   ${ethers.formatUnits(assetsAfter, 18)} ATC  (+${ethers.formatUnits(assetsAfter - assetsBefore, 18)})`);
        console.log(`\nVerify in Polkadot.js: Developer -> Chain state -> assets.account(1, ${stashSs58})`);
    } finally {
        await api.disconnect();
    }
}

main().catch((error) => {
    console.error(`\nFAILED: ${explainRevert(error)}`);
    process.exit(1);
});
