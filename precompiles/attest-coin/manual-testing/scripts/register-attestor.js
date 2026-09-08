'use strict';

// Step 7 — register the attestor, bonding the min bond from the stash.
//
// `registerAttestor` on the attestor-stash precompile dispatches
// `pallet_attestation::register_attestor` with the caller's *mapped* account as
// the origin. That single call:
//
//   * checks the stash's attest-coin balance covers `MinBondRequirement`,
//   * transfers 0.1 CTC from the stash to the attestor account (operating fees),
//   * creates the stash's ledger and moves the bond into the bond pool,
//   * inserts the attestor with status `Idle`.
//
// This has to be an EVM call rather than a Substrate extrinsic. The stash is
// `blake2_256("evm:" || address)` — a hash with no signing key — so the
// extrinsic path is unreachable for it. That is what the stash precompile is for.
//
// The submission goes through the creditcoin CLI's `attestor register`, which
// calls the same precompile. CC_SECRET accepts either a BIP39 phrase or `0x` +
// 64 hex (read as the EVM private key), so STASH_PRIVATE_KEY works directly. The CLI does no validation of its own — it submits
// and reports the revert — so the pre-flight checks below are the value this
// script adds, along with the after-state readout.
//
// Usage:
//   node scripts/register-attestor.js

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');
const { ethers, HDNodeWallet } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { blake2AsU8a, decodeAddress, encodeAddress } = require('@polkadot/util-crypto');

const ENV_PATH = path.resolve(__dirname, '../.env');
require('dotenv').config({ path: ENV_PATH, quiet: true });

const REPO_ROOT = path.resolve(__dirname, '../../../..');
const STASH_ABI_PATH = path.join(
    REPO_ROOT,
    'cli/src/test/blockchain-tests/artifacts/attestor_stash.json',
);

/** Attestor-stash precompile, `PrecompileAt<AddressU64<4052>>` — 4052 == 0xfd4. */
const ATTESTOR_STASH_PRECOMPILE = '0x0000000000000000000000000000000000000fd4';
/** `ATTEST_COIN_ASSET_ID` in runtime/src/lib.rs. */
const ATTEST_COIN_ASSET_ID = 1;
/** `SS58Prefix` in runtime/src/lib.rs. */
const SS58_PREFIX = 42;
/** `ONE_TENTH_CTC` in pallets/attestation/src/impls.rs — forwarded to the attestor. */
const ATTESTOR_TOP_UP = 100_000_000_000_000_000n;
/** Built creditcoin CLI; its `attestor register` calls the same precompile. */
const CLI = path.join(REPO_ROOT, 'cli/dist/cli.js');
/** `AttestorStatus` ordering as the precompile encodes it. */
const STATUS = ['Active', 'Idle', 'Waiting', 'Leaving'];

const RPC_URL = process.env.CC3_RPC_URL || 'http://127.0.0.1:9944';
const WS_URL = process.env.CC3_WS_URL || RPC_URL.replace(/^http/, 'ws');
const CHAIN_KEY = BigInt(process.env.CHAIN_KEY || '2');

/** Raw 32-byte AccountId for an EVM address (`HashedAddressMapping<BlakeTwo256>`). */
function mappedAccountId(evmAddress) {
    const payload = new Uint8Array(24);
    payload.set(new TextEncoder().encode('evm:'), 0);
    payload.set(ethers.getBytes(evmAddress), 4);
    return blake2AsU8a(payload, 256);
}

async function assetBalance(api, ss58) {
    const account = await api.query.assets.account(ATTEST_COIN_ASSET_ID, ss58);
    return account.isSome ? BigInt(account.unwrap().balance.toString()) : 0n;
}

async function nativeBalance(api, ss58) {
    return BigInt((await api.query.system.account(ss58)).data.free.toString());
}

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

/** Run a creditcoin CLI subcommand with the stash secret in CC_SECRET. */
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

/** Frontier wraps a failed dispatch in an EVM revert carrying the pallet error name. */
function explainRevert(error) {
    const message = error instanceof Error ? error.message : String(error);
    const pallet = message.match(/message:\s*Some\(\\?"([A-Za-z0-9_]+)\\?"\)/);
    if (pallet) {
        return `reverted with pallet error ${pallet[1]}`;
    }
    const reason = message.match(/reason="((?:\\.|[^"\\])*)"/);
    return reason ? `reverted: ${reason[1]}` : message;
}

const atc = (base) => `${ethers.formatUnits(base, 18)} ATC`;

async function main() {
    const { STASH_ADDRESS, ATTESTOR_SS58 } = process.env;
    const secret = stashSecret();
    if (!ATTESTOR_SS58) {
        throw new Error('ATTESTOR_SS58 is not set — generate it with subkey (step 5.3)');
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
        // The CLI acts as whatever the secret derives to, so a secret that is not
        // this stash would silently register against a different account.
        const stashAddress = stashAddressFrom(secret);
        if (STASH_ADDRESS && stashAddress.toLowerCase() !== STASH_ADDRESS.toLowerCase()) {
            throw new Error(`the stash secret derives to ${stashAddress} but STASH_ADDRESS is ${STASH_ADDRESS}`);
        }
        const stashAccountId = mappedAccountId(stashAddress);
        const stashSs58 = encodeAddress(stashAccountId, SS58_PREFIX);
        const attestorId = ethers.hexlify(decodeAddress(ATTESTOR_SS58));
        const abi = JSON.parse(fs.readFileSync(STASH_ABI_PATH, 'utf8'));
        const precompile = new ethers.Contract(ATTESTOR_STASH_PRECOMPILE, abi, provider);

        const minBond = BigInt((await precompile.getMinBondRequirement(CHAIN_KEY)).toString());
        const liquid = await assetBalance(api, stashSs58);
        const stashCtc = await nativeBalance(api, stashSs58);
        const attestorCtc = await nativeBalance(api, ATTESTOR_SS58);
        const existing = await precompile.getAttestor(CHAIN_KEY, attestorId);

        console.log(`chain key      ${CHAIN_KEY}`);
        console.log(`stash EVM      ${stashAddress}`);
        console.log(`stash SS58     ${stashSs58}`);
        console.log(`attestor       ${ATTESTOR_SS58}`);
        console.log(`attestor id    ${attestorId}`);
        console.log();
        console.log(`min bond       ${atc(minBond)}`);
        console.log(`stash liquid   ${atc(liquid)}`);
        console.log(`stash CTC      ${ethers.formatEther(stashCtc)}`);
        console.log(`attestor CTC   ${ethers.formatEther(attestorCtc)}`);

        if ((await api.query.supportedChains.supportedChains(CHAIN_KEY)).isNone) {
            throw new Error(`chain key ${CHAIN_KEY} is not a supported chain`);
        }
        if (existing.exists) {
            throw new Error(
                `attestor is already registered on chain key ${CHAIN_KEY} ` +
                    `(status ${STATUS[Number(existing.status)] ?? existing.status})`,
            );
        }
        // The pallet rejects `attestor_id == stash` outright.
        if (attestorId.toLowerCase() === ethers.hexlify(stashAccountId).toLowerCase()) {
            throw new Error('the attestor account and the stash must be different accounts');
        }
        if (liquid < minBond) {
            throw new Error(
                `stash holds ${atc(liquid)} but the min bond is ${atc(minBond)} — deposit more (step 6)`,
            );
        }
        // Registration forwards 0.1 CTC to the attestor with `KeepAlive`, on top
        // of the gas this transaction costs.
        if (stashCtc <= ATTESTOR_TOP_UP) {
            throw new Error(
                `stash has ${ethers.formatEther(stashCtc)} CTC; registration forwards ` +
                    `${ethers.formatEther(ATTESTOR_TOP_UP)} CTC to the attestor and still needs gas ` +
                    '— top it up (step 5.2)',
            );
        }
        // Default on a dev chain is OpenToAny, so this normally passes silently.
        const policy = (await api.query.attestation.chainElectionPolicy(CHAIN_KEY)).toString();
        if (policy === 'AuthorizedOnly') {
            // `AuthorizedAttestors` is a ValueQuery map of `()`, so a direct lookup
            // decodes to `Null` for present and absent keys alike — membership has
            // to be tested by key existence, not by the returned value.
            const entries = await api.query.attestation.authorizedAttestors.entries(CHAIN_KEY);
            const authorized = entries.some(([key]) => key.args[1].toString() === ATTESTOR_SS58);
            if (!authorized) {
                throw new Error(
                    `chain key ${CHAIN_KEY} is AuthorizedOnly and ${ATTESTOR_SS58} is not authorized ` +
                        '— sudo attestation.authorizeAttestor first',
                );
            }
        }

        cli(
            ['attestor', 'register', '-a', ATTESTOR_SS58, '-c', String(CHAIN_KEY), '-u', WS_URL],
            secret,
        );

        const info = await precompile.getAttestor(CHAIN_KEY, attestorId);
        const ledger = await precompile.getLedgerByAddress(stashAddress);

        console.log();
        console.log(`status         ${STATUS[Number(info.status)] ?? info.status}`);
        console.log(`stash on entry ${info.stash === ethers.hexlify(stashAccountId) ? 'matches ours' : info.stash}`);
        console.log(`ledger active  ${atc(ledger.active)}   (bonded)`);
        console.log(`ledger total   ${atc(ledger.totalStaked)}`);
        console.log(`stash liquid   ${atc(await assetBalance(api, stashSs58))}`);
        console.log(`attestor CTC   ${ethers.formatEther(await nativeBalance(api, ATTESTOR_SS58))}`);
        console.log(`
The attestor is Idle. Starting the attestor process (step 8) makes it submit
attest() itself, moving it to Waiting; the next epoch election promotes it to
Active.`);
    } finally {
        await api.disconnect();
    }
}

main().catch((error) => {
    console.error(`\nFAILED: ${explainRevert(error)}`);
    process.exit(1);
});
