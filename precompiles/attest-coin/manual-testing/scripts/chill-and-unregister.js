'use strict';

// Step 10 — chill, then unregister, both through the attestor-stash precompile.
//
// This wraps the creditcoin CLI rather than calling the precompile directly:
// `attestor chill` and `attestor unregister` already go through
// `0x…0fd4` (see cli/src/lib/attestor/precompile.ts), and `chill` additionally
// polls until the attestor reaches Idle — which is the ordering constraint here,
// since `unregister_attestor` rejects anything but Idle.
//
// Chilling an Active attestor schedules the exit (`Leaving`) and only completes
// at the next election, so the wait is real, not cosmetic.
//
// CC_SECRET takes either a BIP39 phrase or `0x` + 64 hex (read as the EVM private
// key), so STASH_PRIVATE_KEY from .env works directly.
//
// Usage:
//   node scripts/chill-and-unregister.js
//   node scripts/chill-and-unregister.js --chill-only

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const { ethers, HDNodeWallet } = require('ethers');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const CLI = path.join(REPO_ROOT, 'cli/dist/cli.js');
const STASH_ABI = path.join(REPO_ROOT, 'cli/src/test/blockchain-tests/artifacts/attestor_stash.json');

const ATTESTOR_STASH_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';
/** `AttestorStatus` ordering as the precompile encodes it. */
const STATUS = ['Active', 'Idle', 'Waiting', 'Leaving'];

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const WS_URL = process.env.CC3_WS_URL || RPC_URL.replace(/^http/, 'ws');
const CHAIN_KEY = process.env.CHAIN_KEY || '2';

/**
 * The stash secret for the CLI. It accepts either form, so the raw EVM private
 * key `new-stash.js` writes is enough — the mnemonic is only a fallback.
 */
function stashSecret() {
    const secret = process.env.STASH_PRIVATE_KEY || process.env.STASH_MNEMONIC;
    if (!secret) {
        throw new Error('neither STASH_PRIVATE_KEY nor STASH_MNEMONIC is set — run scripts/new-stash.js (step 5.2)');
    }
    return secret;
}

/** EVM address the CLI will act as, derived the same way the CLI derives it. */
function stashAddressFrom(secret) {
    return /^0x[0-9a-fA-F]{64}$/.test(secret)
        ? new ethers.Wallet(secret).address
        : HDNodeWallet.fromPhrase(secret).address;
}

function fail(message) {
    console.error(`\nFAILED: ${message}`);
    process.exit(1);
}

/** Run a creditcoin CLI subcommand with the stash phrase in CC_SECRET. */
function cli(args, secret) {
    console.log(`\n$ creditcoin ${args.join(' ')}`);
    const result = spawnSync(process.execPath, [CLI, ...args, '--no-input'], {
        stdio: 'inherit',
        env: { ...process.env, CC_SECRET: secret },
    });
    if (result.status !== 0) {
        fail(`creditcoin ${args[0]} ${args[1]} exited with ${result.status}`);
    }
}

async function main() {
    const chillOnly = process.argv.includes('--chill-only');
    const { STASH_ADDRESS, ATTESTOR_SS58 } = process.env;
    const secret = stashSecret();

    if (!ATTESTOR_SS58) {
        fail('ATTESTOR_SS58 is not set — see step 5.3');
    }
    if (!fs.existsSync(CLI)) {
        fail(`no built CLI at ${CLI}\n  build it with: cd cli && yarn install && yarn build`);
    }

    // The CLI acts as whatever the secret derives to, so a secret that is not this
    // stash would silently operate on a different (probably empty) account.
    const derived = stashAddressFrom(secret);
    if (STASH_ADDRESS && derived.toLowerCase() !== STASH_ADDRESS.toLowerCase()) {
        fail(`the stash secret derives to ${derived} but STASH_ADDRESS is ${STASH_ADDRESS}`);
    }

    const provider = new ethers.JsonRpcProvider(RPC_URL);
    const precompile = new ethers.Contract(
        ATTESTOR_STASH_PRECOMPILE,
        JSON.parse(fs.readFileSync(STASH_ABI, 'utf8')),
        provider,
    );
    const attestorId = ethers.hexlify(require('@polkadot/util-crypto').decodeAddress(ATTESTOR_SS58));

    const before = await precompile.getAttestor(BigInt(CHAIN_KEY), attestorId);
    if (!before.exists) {
        fail(`${ATTESTOR_SS58} is not registered on chain key ${CHAIN_KEY}`);
    }
    const status = STATUS[Number(before.status)] ?? String(before.status);

    console.log(`chain key      ${CHAIN_KEY}`);
    console.log(`stash          ${derived}`);
    console.log(`attestor       ${ATTESTOR_SS58}`);
    console.log(`status         ${status}`);

    // `chill` exits non-zero when the attestor is already Idle, so skip it rather
    // than treating an already-chilled attestor as a failure.
    if (status === 'Idle') {
        console.log('\nalready Idle — skipping chill');
    } else {
        cli(['attestor', 'chill', '-c', CHAIN_KEY, '-a', ATTESTOR_SS58, '-u', WS_URL], secret);
    }

    if (chillOnly) {
        console.log('\n--chill-only: not unregistering.');
        return;
    }

    cli(['attestor', 'unregister', '-a', ATTESTOR_SS58, '-c', CHAIN_KEY, '-u', WS_URL], secret);

    const after = await precompile.getAttestor(BigInt(CHAIN_KEY), attestorId);
    const ledger = await precompile.getLedgerByAddress(derived);
    console.log();
    console.log(`registered     ${after.exists}`);
    console.log(`ledger active  ${ethers.formatUnits(ledger.active, 18)} ATC`);
    console.log(`unlocking      ${ledger.unlockingChunks} chunk(s), ${ethers.formatUnits(ledger.totalStaked, 18)} ATC total`);
    console.log(`withdrawable   ${ethers.formatUnits(ledger.withdrawable, 18)} ATC`);
    console.log('\nBond is unlocking; withdraw it in step 11 once the era has elapsed.');
}

main().catch((error) => fail(error.message));
