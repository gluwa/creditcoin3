require('dotenv').config(); // Load environment variables from .env
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { hexToU8a } = require('@polkadot/util');
const fs = require('fs');
const readline = require('readline');

// Flag handling
const IS_DEV = process.argv.includes('--dev');

const BATCH_SIZE = 100;
const MAX_RETRIES = 10;
// Decrease the retry delay when running with --dev
const RETRY_DELAY_MS = IS_DEV ? 6000 : 15000;
// Wait at least 30s between batch submissions so each batch settles first.
const BATCH_DELAY_MS = 30000;

function parseArgs() {
    const args = process.argv.slice(2);
    const result = {};
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--file' && args[i + 1]) result.file = args[++i];
        else if (args[i] === '--chain-key' && args[i + 1]) result.chainKey = args[++i];
        else if (args[i] === '--rpc' && args[i + 1]) result.rpc = args[++i];
        else if (args[i] === '--keystore' && args[i + 1]) result.keystore = args[++i];
    }
    return result;
}

async function delay(ms) {
    return new Promise((res) => setTimeout(res, ms));
}

// --- Resume support ---------------------------------------------------------
// Progress is computed from on-chain state (the source of truth) rather than a
// local file, so a crashed/interrupted/concurrent run can be safely re-run:
// checkpoints already present on-chain are skipped, only the missing ones are
// imported. Checkpoints live in the attestation.checkpoints double map keyed by
// (chainKey, block_number) -> digest.
//
// Returns a Map<blockNumber(string), onChainDigestHex(string)> for the entries
// that already exist on-chain.
async function fetchExistingDigests(api, chainKey, entries, chunkSize = 1000) {
    const existing = new Map();
    for (let i = 0; i < entries.length; i += chunkSize) {
        const chunk = entries.slice(i, i + chunkSize);
        const keys = chunk.map((e) => [chainKey, e.blockNumber]);
        const results = await api.query.attestation.checkpoints.multi(keys);
        results.forEach((res, idx) => {
            if (res.isSome) {
                existing.set(chunk[idx].blockNumber, res.unwrap().toHex().toLowerCase());
            }
        });
        console.log(`  checked ${Math.min(i + chunkSize, entries.length)}/${entries.length} checkpoints on-chain…`);
    }
    return existing;
}

// Prompt for a password without echoing it to the terminal.
function promptPassword(question) {
    return new Promise((resolve) => {
        const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
        const onData = () => rl.output.write('\x1B[2K\r' + question);
        rl.input.on('data', onData);
        rl.question(question, (answer) => {
            rl.input.off('data', onData);
            rl.close();
            process.stdout.write('\n');
            resolve(answer);
        });
    });
}

// Build the signing keypair from either an encrypted keystore JSON (prompts for
// the password unless KEYSTORE_PASSWORD is set) or a raw MNEMONIC/seed suri.
async function resolveSigner(keyring, cliArgs) {
    const keystoreFile = cliArgs.keystore || process.env.KEYSTORE_FILE;
    if (keystoreFile) {
        const json = JSON.parse(fs.readFileSync(keystoreFile, 'utf8'));
        const password = process.env.KEYSTORE_PASSWORD || (await promptPassword('Keystore password: '));
        const pair = keyring.addFromJson(json);
        try {
            pair.decodePkcs8(password);
        } catch (err) {
            throw new Error(`Failed to unlock keystore (wrong password or unsupported format): ${err.message}`);
        }
        return pair;
    }

    const mnemonic = process.env.MNEMONIC;
    if (!mnemonic) {
        throw new Error('No signer configured. Use --keystore <file> or set MNEMONIC in the environment');
    }
    return keyring.addFromUri(mnemonic);
}

async function main() {
    if (IS_DEV) {
        console.log('Running in DEV mode: RETRY_DELAY_MS set to 6000ms');
    }

    const cliArgs = parseArgs();

    // Resolve config: CLI args take priority over env vars
    const csvFile = cliArgs.file || process.env.CHECKPOINTS_FILE;
    if (!csvFile) {
        throw new Error('CSV file not specified. Use --file <path> or set CHECKPOINTS_FILE env var');
    }

    const destinationChain = cliArgs.rpc || process.env.DESTINATION_CHAIN;
    if (!destinationChain) {
        throw new Error('RPC endpoint not specified. Use --rpc <url> or set DESTINATION_CHAIN env var');
    }

    const chainKey = cliArgs.chainKey || process.env.CHAIN_KEY_ON_DESTINATION;
    if (!chainKey) {
        throw new Error('Chain key not specified. Use --chain-key <key> or set CHAIN_KEY_ON_DESTINATION env var');
    }

    // Get api and keyring
    const provider = new WsProvider(destinationChain);
    const api = await ApiPromise.create({ provider });

    // sudo.wrap is ON by default; set USE_SUDO=0 (or false) to sign
    // import_checkpoints directly via the operator origin.
    const useSudo = process.env.USE_SUDO !== '0' && process.env.USE_SUDO !== 'false';

    const keyring = new Keyring({ type: 'sr25519' });
    const signer = await resolveSigner(keyring, cliArgs);
    console.log(`Signer address: ${signer.address} (USE_SUDO=${useSudo})`);

    // Parse CSV: each line is "block_number,digest_hash"
    const rawData = fs.readFileSync(csvFile, 'utf8');
    const lines = rawData.trim().split('\n');

    // Skip header line if present (starts with non-numeric)
    const dataLines = lines.filter((line) => {
        const firstChar = line.trim()[0];
        return firstChar >= '0' && firstChar <= '9';
    });

    const entries = dataLines.map((line) => {
        const [blockNumber, digestHex] = line.trim().split(',');
        return { blockNumber: blockNumber.trim(), digestHex: digestHex.trim() };
    });

    // Reversing entries so that we insert them from newest to oldest
    const reversedEntries = entries.reverse();

    console.log(`Loaded ${reversedEntries.length} checkpoints from ${csvFile}`);

    // Compute progress from chain state: skip checkpoints already imported so a
    // crashed/interrupted/concurrent run can be safely re-run.
    console.log('Computing progress from chain state…');
    const existing = await fetchExistingDigests(api, chainKey, reversedEntries);

    let mismatches = 0;
    const pending = [];
    for (const e of reversedEntries) {
        const onChain = existing.get(e.blockNumber);
        if (onChain === undefined) {
            pending.push(e);
        } else if (onChain !== e.digestHex.toLowerCase()) {
            mismatches++;
            console.warn(`⚠️ Digest mismatch at block ${e.blockNumber}: on-chain ${onChain} vs CSV ${e.digestHex} (leaving on-chain value untouched)`);
        }
    }

    console.log(
        `${existing.size} already on-chain, ${pending.length} pending to import` +
            (mismatches ? `, ${mismatches} digest mismatches (see warnings above)` : ''),
    );

    if (pending.length === 0) {
        if (mismatches > 0) {
            console.error(
                `❌ Nothing to import, but ${mismatches} digest mismatch(es) between CSV and chain ` +
                    `(see warnings above). On-chain values were left untouched. ` +
                    `Resolve the conflict before treating this run as successful.`,
            );
            process.exit(2);
        }
        console.log('✅ Nothing to import — all checkpoints already present on-chain.');
        process.exit(0);
    }

    const totalBatches = Math.ceil(pending.length / BATCH_SIZE);
    for (let i = 0; i < pending.length; i += BATCH_SIZE) {
        const batch = pending.slice(i, i + BATCH_SIZE);

        const checkpointVec = batch.map(({ blockNumber, digestHex }) => {
            return api.createType('AttestorPrimitivesAttestationCheckpoint', {
                digest: hexToU8a(digestHex),
                // Use bigint to avoid precision loss when block numbers exceed Number.MAX_SAFE_INTEGER
                block_number: BigInt(blockNumber),
            });
        });

        const boundedVec = api.createType('BoundedVec<AttestorPrimitivesAttestationCheckpoint, 100>', checkpointVec);

        const call = api.tx.attestation.importCheckpoints(chainKey, boundedVec);
        const tx = useSudo ? api.tx.sudo.sudo(call) : call;

        let attempt = 0;
        while (attempt < MAX_RETRIES) {
            console.log(`Submitting batch ${Math.floor(i / BATCH_SIZE) + 1}/${totalBatches}, attempt ${attempt + 1}...`);
            try {
                await new Promise((resolve, reject) => {
                    let unsub;
                    tx
                        .signAndSend(signer, (result) => {
                            if (result.status.isInBlock) {
                                console.log(`📦 Batch included in block: ${result.status.asInBlock}`);
                                if (unsub) unsub();
                                resolve();
                            } else if (result.isError) {
                                console.error('❌ Transaction error reported');
                                if (unsub) unsub();
                                reject(new Error(`Transaction error reported: ${result.toHuman()}`));
                            }
                        })
                        .then((u) => {
                            unsub = u;
                        })
                        .catch(reject);
                });
                break; // exit retry loop on success
            } catch (err) {
                console.error(`⚠️ Error submitting batch: ${err.message}`);
                attempt++;
                if (attempt >= MAX_RETRIES) {
                    throw new Error(`❌ Failed to submit batch after ${MAX_RETRIES} attempts`);
                }
                await delay(RETRY_DELAY_MS);
            }
        }

        // Wait at least 30s before submitting the next batch.
        await delay(BATCH_DELAY_MS);
    }

    if (mismatches > 0) {
        console.error(
            `❌ All pending checkpoint batches submitted, but ${mismatches} digest mismatch(es) ` +
                `between CSV and chain were detected (see warnings above). On-chain values for the ` +
                `mismatched blocks were left untouched. Resolve the conflict before treating this run ` +
                `as fully successful.`,
        );
        process.exit(2);
    }

    console.log('✅ All checkpoint batches submitted.');
    process.exit(0);
}

main().catch((err) => {
    console.error('❌ Error:', err);
    process.exit(1);
});
