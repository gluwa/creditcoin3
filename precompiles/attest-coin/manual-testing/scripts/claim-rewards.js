'use strict';

// Step 9 — wait for the first committed attestation, then claim the reward.
//
// Rewards accrue to the *stash*, not the attestor account: `reward_commit_signers`
// credits `RewardPerEligibleSigner` (1 ATC) per eligible signer on every
// successful `commit_attestation`.
//
// The claim is self-authorizing. Our stash is the `AddressMapping` image of an
// EVM address — a blake2 hash with no sr25519 key — so no signature over it can
// exist; the precompile accepts the EVM caller directly when
// `into_account_id(msg.sender) == stash`, and the signature args are ignored.
// `ClaimNonce` still bounds replay.
//
// Usage:
//   node scripts/claim-rewards.js               # wait, then claim everything accrued
//   node scripts/claim-rewards.js --watch-only  # wait and report, claim nothing
//   node scripts/claim-rewards.js --timeout 600

const fs = require('fs');
const path = require('path');
const { ethers } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { blake2AsU8a, encodeAddress } = require('@polkadot/util-crypto');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const ARTIFACTS = path.join(REPO_ROOT, 'cli/src/test/blockchain-tests/artifacts');

/** Attest-coin precompile, `AddressU64<4053>` — also the ERC-20 treasury. */
const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';
const ATTEST_COIN_ASSET_ID = 1;
const SS58_PREFIX = 42;
/** `claim` does an sr25519 verify plus an ERC-20 transfer. */
const CLAIM_GAS = 3_000_000;

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const WS_URL = process.env.CC3_WS_URL || RPC_URL.replace(/^http/, 'ws');
const CHAIN_KEY = BigInt(process.env.CHAIN_KEY || '2');

const atc = (v) => `${ethers.formatUnits(v, 18)} ATC`;
const artifact = (name) => JSON.parse(fs.readFileSync(path.join(ARTIFACTS, name), 'utf8'));

function mappedAccountId(evmAddress) {
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(ethers.getBytes(evmAddress), 4);
    return blake2AsU8a(payload, 256);
}

/** `PalletId(*b"att/bond").into_account_truncating()` — literal bytes, not a hash. */
function bondPoolAccountId() {
    const raw = new Uint8Array(32);
    raw.set(new TextEncoder().encode('modlatt/bond'), 0);
    return raw;
}

async function assetBalance(api, account) {
    const entry = await api.query.assets.account(ATTEST_COIN_ASSET_ID, account);
    return entry.isSome ? BigInt(entry.unwrap().balance.toString()) : 0n;
}

/**
 * ERC-20 the treasury must retain: every unit of attest coin outside the bond
 * pool can be `withdraw`n back, so a claim may only spend what sits above that.
 */
async function withdrawableBacking(api) {
    const asset = await api.query.assets.asset(ATTEST_COIN_ASSET_ID);
    if (asset.isNone) {
        return 0n;
    }
    const supply = BigInt(asset.unwrap().supply.toString());
    const pool = await assetBalance(api, encodeAddress(bondPoolAccountId(), SS58_PREFIX));
    return supply > pool ? supply - pool : 0n;
}

async function feeOverrides(provider, gasLimit) {
    const latest = await provider.getBlock('latest');
    const baseFee = latest?.baseFeePerGas ?? 1_000_000_000n;
    const priority = (await provider.getFeeData()).maxPriorityFeePerGas ?? 1_000_000_000n;
    return { gasLimit, maxFeePerGas: baseFee * 10n + priority, maxPriorityFeePerGas: priority };
}

function explainRevert(error) {
    const message = error instanceof Error ? error.message : String(error);
    const pallet = message.match(/message:\s*Some\(\\?"([A-Za-z0-9_]+)\\?"\)/);
    if (pallet) {
        return `reverted with pallet error ${pallet[1]}`;
    }
    const reason = message.match(/reason="((?:\\.|[^"\\])*)"/);
    return reason ? `reverted: ${reason[1]}` : message;
}

/**
 * Block until the stash has accrued something, reporting the attestation events
 * as they land. Resolves with the accrued amount.
 */
function waitForReward(api, precompile, stashB32, timeoutMs) {
    return new Promise((resolve, reject) => {
        let unsub;
        const timer = setTimeout(() => {
            if (unsub) unsub();
            reject(new Error(`no reward accrued within ${timeoutMs / 1000}s — is the attestor Active and committing?`));
        }, timeoutMs);

        const finish = (value) => {
            clearTimeout(timer);
            if (unsub) unsub();
            resolve(value);
        };

        api.query.system
            .events(async (records) => {
                let interesting = false;
                for (const { event } of records) {
                    const key = `${event.section}.${event.method}`;
                    if (key === 'attestation.BlockAttested') {
                        const [chainKey, height] = event.data;
                        console.log(`  ${key}  chain=${chainKey} height=${height}`);
                        interesting = true;
                    } else if (key === 'attestCoinRewards.CommitSignersRewarded') {
                        // Named-field events still decode positionally here — polkadot-js
                        // hands back a tuple, not an object keyed by field name.
                        const [ck, signers, per] = event.data;
                        console.log(`  ${key}  chain=${ck} signers=${signers} per=${atc(BigInt(per.toString()))}`);
                        interesting = true;
                    } else if (key === 'attestCoinRewards.RewardSkippedNoStash') {
                        console.log(`  ${key}  ${event.data.toString()}  <- desync, investigate`);
                    }
                }
                if (!interesting) {
                    return;
                }
                const accrued = BigInt((await precompile.accrued(stashB32)).toString());
                if (accrued > 0n) {
                    finish(accrued);
                }
            })
            .then((u) => {
                unsub = u;
            })
            .catch(reject);
    });
}

async function main() {
    const argv = process.argv.slice(2);
    const watchOnly = argv.includes('--watch-only');
    const timeoutIdx = argv.indexOf('--timeout');
    const timeoutMs = (timeoutIdx >= 0 ? Number(argv[timeoutIdx + 1]) : 300) * 1000;

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
        const stashAccountId = mappedAccountId(stash.address);
        const stashSs58 = encodeAddress(stashAccountId, SS58_PREFIX);
        const stashB32 = ethers.hexlify(stashAccountId);
        const precompile = new ethers.Contract(ATTEST_COIN_PRECOMPILE, artifact('attest_coin.json'), stash);
        const token = new ethers.Contract(ATTESTCOIN_ERC20, artifact('MockAttestToken.json').abi, stash);

        console.log(`chain key      ${CHAIN_KEY}`);
        console.log(`stash SS58     ${stashSs58}   (rewards accrue here)`);
        console.log(`attestor       ${process.env.ATTESTOR_SS58 || '(not set)'}`);

        let accrued = BigInt((await precompile.accrued(stashB32)).toString());
        console.log(`accrued now    ${atc(accrued)}`);

        if (accrued === 0n) {
            console.log(`\nwaiting for the first committed attestation (timeout ${timeoutMs / 1000}s)...`);
            accrued = await waitForReward(api, precompile, stashB32, timeoutMs);
        }
        console.log(`\naccrued        ${atc(accrued)}`);

        // Cross-check the precompile view against runtime storage.
        const stored = BigInt((await api.query.attestCoinRewards.accrued(stashSs58)).toString());
        console.log(`storage agrees ${stored === accrued}`);

        if (watchOnly) {
            console.log('\n--watch-only: not claiming.');
            return;
        }

        const treasury = BigInt((await token.balanceOf(ATTEST_COIN_PRECOMPILE)).toString());
        const backing = await withdrawableBacking(api);
        console.log(`treasury       ${atc(treasury)}  (must keep ${atc(backing)} as deposit backing)`);
        if (treasury < accrued + backing) {
            throw new Error(
                `treasury holds ${atc(treasury)} but the claim needs ${atc(accrued + backing)} ` +
                    '— mint more with scripts/fund-erc20.js precompile <amount>',
            );
        }

        const nonce = BigInt((await api.query.attestCoinRewards.claimNonce(stashSs58)).toString());
        const erc20Before = BigInt((await token.balanceOf(stash.address)).toString());
        console.log(`claim nonce    ${nonce}`);

        console.log(`\nclaiming ${atc(accrued)} to ${stash.address}...`);
        const receipt = await (
            await precompile.claim(
                stashB32,
                nonce,
                CHAIN_KEY,
                accrued,
                stash.address,
                ethers.ZeroHash,
                ethers.ZeroHash,
                await feeOverrides(provider, CLAIM_GAS),
            )
        ).wait();
        console.log(`claimed        tx ${receipt.hash}`);

        const erc20After = BigInt((await token.balanceOf(stash.address)).toString());
        console.log();
        console.log(`stash ERC-20   ${atc(erc20After)}   (+${atc(erc20After - erc20Before)})`);
        console.log(`accrued        ${atc(await precompile.accrued(stashB32))}`);
        console.log(`claim nonce    ${(await api.query.attestCoinRewards.claimNonce(stashSs58)).toString()}`);
    } finally {
        await api.disconnect();
    }
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error(`\nFAILED: ${explainRevert(error)}`);
        process.exit(1);
    });
