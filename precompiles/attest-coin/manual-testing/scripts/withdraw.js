'use strict';

// Step 11 — wait out the unbonding period and take the bond back to ERC-20.
//
// Two moves, the first unblocking the second:
//
//   1. `attestor withdraw-unbonded` (CLI, stash precompile) returns elapsed
//      chunks from the bond pool to the stash's liquid pallet-assets balance,
//      and reaps the ledger once nothing is left bonded.
//   2. `withdraw` on the attest-coin precompile burns that liquid attest coin
//      and sends the same amount of ERC-20 to the caller. No CLI equivalent —
//      the attest-coin group only exposes read-only helpers.
//
// Unregistering is step 10's job; this script only checks it happened, since
// without it there is no unlocking chunk to withdraw. Both stages are skipped if
// already done, so re-running is safe and the era wait can be resumed later.
//
// Usage:
//   node scripts/withdraw.js
//   node scripts/withdraw.js --dry-run       # report what it would do, submit nothing
//   node scripts/withdraw.js --timeout 900   # seconds to wait for the era (default 600)

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const { ethers, HDNodeWallet } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { blake2AsU8a, decodeAddress, encodeAddress } = require('@polkadot/util-crypto');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const CLI = path.join(REPO_ROOT, 'cli/dist/cli.js');
const ARTIFACTS = path.join(REPO_ROOT, 'cli/src/test/blockchain-tests/artifacts');

const ATTESTOR_STASH_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';
const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';
const ATTEST_COIN_ASSET_ID = 1;
const SS58_PREFIX = 42;
/** `pallet_assets::burn` dispatch plus the ERC-20 transfer subcall. */
const WITHDRAW_GAS = 8_000_000;
const STATUS = ['Active', 'Idle', 'Waiting', 'Leaving'];

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const WS_URL = process.env.CC3_WS_URL || RPC_URL.replace(/^http/, 'ws');
const CHAIN_KEY = process.env.CHAIN_KEY || '2';

const atc = (v) => `${ethers.formatUnits(v, 18)} ATC`;
const artifact = (name) => JSON.parse(fs.readFileSync(path.join(ARTIFACTS, name), 'utf8'));
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function mappedAccountId(evmAddress) {
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(ethers.getBytes(evmAddress), 4);
    return blake2AsU8a(payload, 256);
}

/** The CLI accepts either secret form, so the raw EVM private key is enough. */
function stashSecret() {
    const secret = process.env.STASH_PRIVATE_KEY || process.env.STASH_MNEMONIC;
    if (!secret) {
        throw new Error('neither STASH_PRIVATE_KEY nor STASH_MNEMONIC is set — run scripts/new-stash.js');
    }
    return secret;
}

function stashAddressFrom(secret) {
    return /^0x[0-9a-fA-F]{64}$/.test(secret)
        ? new ethers.Wallet(secret).address
        : HDNodeWallet.fromPhrase(secret).address;
}

function cli(args, secret) {
    console.log(`\n$ creditcoin ${args.join(' ')}`);
    const result = spawnSync(process.execPath, [CLI, ...args, '--no-input'], {
        stdio: 'inherit',
        env: { ...process.env, CC_SECRET: secret },
    });
    if (result.status !== 0) {
        throw new Error(`creditcoin ${args[0]} ${args[1]} exited with ${result.status}`);
    }
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

async function currentEra(api) {
    const era = await api.query.staking.currentEra();
    return era.isSome ? Number(era.unwrap().toString()) : 0;
}

async function main() {
    const argv = process.argv.slice(2);
    const idx = argv.indexOf('--timeout');
    const timeoutMs = (idx >= 0 ? Number(argv[idx + 1]) : 600) * 1000;
    const dryRun = argv.includes('--dry-run');

    const secret = stashSecret();
    const stashAddress = stashAddressFrom(secret);
    const { STASH_ADDRESS, ATTESTOR_SS58, ATTESTCOIN_ERC20 } = process.env;
    if (STASH_ADDRESS && stashAddress.toLowerCase() !== STASH_ADDRESS.toLowerCase()) {
        throw new Error(`the stash secret derives to ${stashAddress} but STASH_ADDRESS is ${STASH_ADDRESS}`);
    }
    if (!ATTESTOR_SS58) {
        throw new Error('ATTESTOR_SS58 is not set — see step 5.3');
    }
    if (!ATTESTCOIN_ERC20) {
        throw new Error('ATTESTCOIN_ERC20 is not set — run scripts/deploy-erc20.js (step 3)');
    }
    if (!fs.existsSync(CLI)) {
        throw new Error(`no built CLI at ${CLI}\n  build it with: cd cli && yarn install && yarn build`);
    }

    const provider = new ethers.JsonRpcProvider(RPC_URL);
    const api = await ApiPromise.create({
        provider: new WsProvider(WS_URL, 1_000, {}, 20_000),
        noInitWarn: true,
        throwOnConnect: true,
    });

    try {
        const stashSs58 = encodeAddress(mappedAccountId(stashAddress), SS58_PREFIX);
        const stashPrecompile = new ethers.Contract(
            ATTESTOR_STASH_PRECOMPILE,
            artifact('attestor_stash.json'),
            provider,
        );
        const attestCoin = new ethers.Contract(
            ATTEST_COIN_PRECOMPILE,
            artifact('attest_coin.json'),
            new ethers.Wallet(process.env.STASH_PRIVATE_KEY || secret, provider),
        );
        const token = new ethers.Contract(ATTESTCOIN_ERC20, artifact('MockAttestToken.json').abi, provider);
        const attestorId = ethers.hexlify(decodeAddress(ATTESTOR_SS58));

        const liquidOf = async () => {
            const e = await api.query.assets.account(ATTEST_COIN_ASSET_ID, stashSs58);
            return e.isSome ? BigInt(e.unwrap().balance.toString()) : 0n;
        };

        console.log(`stash          ${stashAddress}`);
        console.log(`stash SS58     ${stashSs58}`);
        console.log(`era            ${await currentEra(api)}`);

        // Precondition, not a stage: `unregister_attestor` is what creates the
        // unlocking chunk, so nothing here can free anything until step 10 ran.
        const attestor = await stashPrecompile.getAttestor(BigInt(CHAIN_KEY), attestorId);
        if (attestor.exists) {
            const status = STATUS[Number(attestor.status)] ?? String(attestor.status);
            throw new Error(
                `${ATTESTOR_SS58} is still registered (status ${status}) — run step 10 ` +
                    '(scripts/chill-and-unregister.js) first',
            );
        }

        // ---- 1. wait for the unbonding period, then withdraw-unbonded ----
        let ledger = await stashPrecompile.getLedgerByAddress(stashAddress);
        if (ledger.exists && BigInt(ledger.totalStaked) > 0n) {
            console.log(`\nledger total   ${atc(ledger.totalStaked)}`);
            console.log(`unlocking      ${ledger.unlockingChunks} chunk(s)`);
            const deadline = Date.now() + timeoutMs;
            while (BigInt(ledger.withdrawable) === 0n && !dryRun) {
                if (Date.now() > deadline) {
                    throw new Error(
                        `nothing withdrawable after ${timeoutMs / 1000}s — the unbonding period is ` +
                            'BondingDuration (2) eras; re-run later or raise --timeout',
                    );
                }
                console.log(`  era ${await currentEra(api)}: withdrawable ${atc(ledger.withdrawable)}, waiting...`);
                await sleep(15_000);
                ledger = await stashPrecompile.getLedgerByAddress(stashAddress);
            }
            console.log(`withdrawable   ${atc(ledger.withdrawable)} at era ${await currentEra(api)}`);
            if (dryRun) {
                console.log('would run: creditcoin attestor withdraw-unbonded');
            } else {
                cli(['attestor', 'withdraw-unbonded', '-u', WS_URL], secret);
            }
        } else {
            console.log('\nno bond left in the ledger — skipping withdraw-unbonded');
        }

        // ---- 2. burn liquid attest coin back to ERC-20 ----
        const liquid = await liquidOf();
        const erc20Before = BigInt((await token.balanceOf(stashAddress)).toString());
        console.log(`\nstash liquid   ${atc(liquid)}`);
        console.log(`stash ERC-20   ${atc(erc20Before)}`);

        if (liquid === 0n) {
            // In a dry run nothing was actually returned, so project what stage 2
            // would have freed rather than reporting a misleading "nothing to do".
            const projected = ledger.exists ? BigInt(ledger.withdrawable) : 0n;
            console.log(
                dryRun && projected > 0n
                    ? `\nwould then call attest-coin withdraw(${atc(projected)}) once withdraw-unbonded returns it.`
                    : '\nnothing liquid to withdraw — done.',
            );
            return;
        }
        const treasury = BigInt((await token.balanceOf(ATTEST_COIN_PRECOMPILE)).toString());
        if (treasury < liquid) {
            throw new Error(`treasury holds ${atc(treasury)}, cannot pay out ${atc(liquid)}`);
        }

        if (dryRun) {
            console.log(`\nwould call attest-coin withdraw(${atc(liquid)}) — nothing submitted.`);
            return;
        }

        console.log(`\nwithdrawing ${atc(liquid)} to ERC-20...`);
        const receipt = await (
            await attestCoin.withdraw(liquid, await feeOverrides(provider, WITHDRAW_GAS))
        ).wait();
        console.log(`withdrew       tx ${receipt.hash}`);

        const erc20After = BigInt((await token.balanceOf(stashAddress)).toString());
        console.log();
        console.log(`stash liquid   ${atc(await liquidOf())}`);
        console.log(`stash ERC-20   ${atc(erc20After)}   (+${atc(erc20After - erc20Before)})`);
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
