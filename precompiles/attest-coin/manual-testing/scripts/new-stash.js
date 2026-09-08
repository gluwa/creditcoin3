'use strict';

// Generate a fresh EVM account to use as the attestor stash.
//
// The stash should be a brand new account rather than one of the chainspec's
// endowed dev keys: step 14 checks that claimed rewards plus the returned bond
// add up, which is only clean if the account starts empty.
//
// Beyond the keypair this prints the stash's *mapped Substrate account* —
// `blake2_256("evm:" || address)`, the runtime's `HashedAddressMapping`. That is
// the account `pallet-attestation` actually bonds from and accrues rewards to,
// so it is what you query in Polkadot.js chain state (`system.account`,
// `assets.account(1, …)`, `attestation.ledger(…)`).
//
// Usage:
//   node scripts/new-stash.js            # refuses if a stash is already in .env
//   node scripts/new-stash.js --force    # replace the existing one anyway

const fs = require('fs');
const path = require('path');
const { ethers } = require('ethers');
const { blake2AsU8a, encodeAddress } = require('@polkadot/util-crypto');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

/** `SS58Prefix` in runtime/src/lib.rs. */
const SS58_PREFIX = 42;

/**
 * Raw 32-byte Substrate AccountId an EVM address maps to, matching the runtime's
 * `HashedAddressMapping<BlakeTwo256>`: `blake2_256("evm:" || address)`.
 */
function mappedAccountId(evmAddress) {
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(ethers.getBytes(evmAddress), 4);
    return blake2AsU8a(payload, 256);
}

/** Set `key=value` in .env, replacing an existing line or appending a new one. */
function writeEnv(entries) {
    let contents = fs.readFileSync(ENV_PATH, 'utf8');
    for (const [key, value] of Object.entries(entries)) {
        const line = `${key}=${value}`;
        const existing = new RegExp(`^${key}=.*$`, 'm');
        contents = existing.test(contents)
            ? contents.replace(existing, line)
            : `${contents.replace(/\n*$/, '\n')}${line}\n`;
    }
    fs.writeFileSync(ENV_PATH, contents);
}

function main() {
    const force = process.argv.includes('--force');
    const existing = process.env.STASH_PRIVATE_KEY;

    if (existing && !force) {
        throw new Error(
            `.env already has STASH_PRIVATE_KEY (stash ${process.env.STASH_ADDRESS || 'unknown'}).\n` +
                'Re-run with --force to replace it — but any CTC or ATC already funded to the\n' +
                'old stash, and any bond it holds, would be stranded.',
        );
    }

    const wallet = ethers.Wallet.createRandom();
    const accountId = mappedAccountId(wallet.address);

    // The mnemonic is saved too, not just for recovery: the creditcoin CLI's
    // `attestor` commands take the stash as a BIP39 phrase in CC_SECRET and
    // reject a raw hex key, so steps 10 and 11 need this form.
    writeEnv({
        STASH_ADDRESS: wallet.address,
        STASH_PRIVATE_KEY: wallet.privateKey,
        STASH_MNEMONIC: wallet.mnemonic.phrase,
    });

    console.log(`EVM address    ${wallet.address}`);
    console.log(`private key    ${wallet.privateKey}`);
    console.log(`mnemonic       ${wallet.mnemonic.phrase}`);
    console.log();
    console.log(`mapped SS58    ${encodeAddress(accountId, SS58_PREFIX)}`);
    console.log(`mapped hex     ${ethers.hexlify(accountId)}`);
    console.log();
    console.log(`Saved STASH_ADDRESS, STASH_PRIVATE_KEY and STASH_MNEMONIC to ${ENV_PATH}`);
    console.log(`
To fund it with CTC (step 5.2) you do not need the SS58 form — the runtime's
lookup accepts raw EVM addresses, so in Polkadot.js pick the Address20 variant
on the "who" field and paste ${wallet.address} directly.`);
}

try {
    main();
} catch (error) {
    console.error(`\nFAILED: ${error.message}`);
    process.exit(1);
}
