'use strict';

// Step 12 — print the end-state balances as a sanity check.
//
// Read-only. After a full run the stash should hold no bonded and no liquid
// attest coin, with everything sitting in ERC-20:
//
//   ERC-20 == rewards claimed + the bond that round-tripped back out
//
// Usage: node scripts/show-balances.js

const fs = require('fs');
const path = require('path');
const { ethers } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { blake2AsU8a, encodeAddress } = require('@polkadot/util-crypto');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const ARTIFACTS = path.join(REPO_ROOT, 'cli/src/test/blockchain-tests/artifacts');

const ATTESTOR_STASH_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';
const ATTEST_COIN_PRECOMPILE = '0x0000000000000000000000000000000000000fd5';
const ATTEST_COIN_ASSET_ID = 1;
const SS58_PREFIX = 42;

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const WS_URL = process.env.CC3_WS_URL || RPC_URL.replace(/^http/, 'ws');

const atc = (v) => `${ethers.formatUnits(v, 18)} ATC`;
const artifact = (name) => JSON.parse(fs.readFileSync(path.join(ARTIFACTS, name), 'utf8'));
const row = (label, value) => console.log(`  ${label.padEnd(26)} ${value}`);

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

async function main() {
    const { STASH_ADDRESS, ATTESTCOIN_ERC20 } = process.env;
    if (!STASH_ADDRESS) {
        throw new Error('STASH_ADDRESS is not set — run scripts/new-stash.js (step 5.2)');
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
        const stashSs58 = encodeAddress(mappedAccountId(STASH_ADDRESS), SS58_PREFIX);
        const token = new ethers.Contract(ATTESTCOIN_ERC20, artifact('MockAttestToken.json').abi, provider);
        const stashPrecompile = new ethers.Contract(
            ATTESTOR_STASH_PRECOMPILE,
            artifact('attestor_stash.json'),
            provider,
        );

        const liquidEntry = await api.query.assets.account(ATTEST_COIN_ASSET_ID, stashSs58);
        const liquid = liquidEntry.isSome ? BigInt(liquidEntry.unwrap().balance.toString()) : 0n;
        const ledger = await stashPrecompile.getLedgerByAddress(STASH_ADDRESS);
        const accrued = BigInt((await api.query.attestCoinRewards.accrued(stashSs58)).toString());
        const erc20 = BigInt((await token.balanceOf(STASH_ADDRESS)).toString());
        const nonce = (await api.query.attestCoinRewards.claimNonce(stashSs58)).toString();

        console.log(`\nstash ${STASH_ADDRESS}`);
        console.log(`      ${stashSs58}\n`);
        row('ERC-20 (ATC)', atc(erc20));
        row('liquid attest coin', atc(liquid));
        row('ledger exists', ledger.exists);
        if (ledger.exists) {
            row('  active (bonded)', atc(ledger.active));
            row('  unlocking chunks', ledger.unlockingChunks.toString());
            row('  withdrawable', atc(ledger.withdrawable));
        }
        row('unclaimed rewards', atc(accrued));
        row('claims made', nonce);
        row('native CTC', `${ethers.formatEther((await api.query.system.account(stashSs58)).data.free.toString())} CTC`);

        const asset = await api.query.assets.asset(ATTEST_COIN_ASSET_ID);
        const supply = asset.isSome ? BigInt(asset.unwrap().supply.toString()) : 0n;
        const poolEntry = await api.query.assets.account(
            ATTEST_COIN_ASSET_ID,
            encodeAddress(bondPoolAccountId(), SS58_PREFIX),
        );
        const pool = poolEntry.isSome ? BigInt(poolEntry.unwrap().balance.toString()) : 0n;

        console.log('\nchain-wide');
        row('asset 1 total supply', atc(supply));
        row('bond pool holds', atc(pool));
        row('treasury ERC-20', atc(await token.balanceOf(ATTEST_COIN_PRECOMPILE)));

        const done = !ledger.exists && liquid === 0n && accrued === 0n;
        console.log(
            done
                ? '\nFully unwound: nothing bonded, nothing liquid, nothing unclaimed.\n' +
                      `The ${atc(erc20)} above is rewards claimed plus the bond that round-tripped back out.`
                : '\nStill in flight — see the non-zero rows above.',
        );
    } finally {
        await api.disconnect();
    }
}

main()
    .then(() => process.exit(0))
    .catch((error) => {
        console.error(`\nFAILED: ${error.message}`);
        process.exit(1);
    });
