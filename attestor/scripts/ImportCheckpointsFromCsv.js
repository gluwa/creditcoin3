require('dotenv').config(); // Load environment variables from .env
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { hexToU8a } = require('@polkadot/util');
const fs = require('fs');

// Flag handling
const IS_DEV = process.argv.includes('--dev');

const BATCH_SIZE = 100;
const MAX_RETRIES = 10;
// Decrease the retry delay when running with --dev
const RETRY_DELAY_MS = IS_DEV ? 6000 : 15000;

function parseArgs() {
    const args = process.argv.slice(2);
    const result = {};
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--file' && args[i + 1]) result.file = args[++i];
        else if (args[i] === '--chain-key' && args[i + 1]) result.chainKey = args[++i];
        else if (args[i] === '--rpc' && args[i + 1]) result.rpc = args[++i];
    }
    return result;
}

async function delay(ms) {
    return new Promise((res) => setTimeout(res, ms));
}

async function main() {
    if (IS_DEV) {
        console.log('Running in DEV mode: RETRY_DELAY_MS set to 6000ms');
    }

    const cliArgs = parseArgs();

    // Resolve config: CLI args take priority over env vars
    const mnemonic = process.env.MNEMONIC;
    if (!mnemonic) {
        throw new Error('MNEMONIC not found in environment');
    }

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

    const keyring = new Keyring({ type: 'sr25519' });
    const sudo = keyring.addFromUri(mnemonic);
    console.log('Sudo address:', sudo.address);

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

    for (let i = 0; i < reversedEntries.length; i += BATCH_SIZE) {
        const batch = reversedEntries.slice(i, i + BATCH_SIZE);

        const checkpointVec = batch.map(({ blockNumber, digestHex }) => {
            return api.createType('AttestorPrimitivesAttestationCheckpoint', {
                digest: hexToU8a(digestHex),
                // Use bigint to avoid precision loss when block numbers exceed Number.MAX_SAFE_INTEGER
                block_number: BigInt(blockNumber),
            });
        });

        const boundedVec = api.createType('BoundedVec<AttestorPrimitivesAttestationCheckpoint, 100>', checkpointVec);

        const call = api.tx.attestation.importCheckpoints(chainKey, boundedVec);
        const sudoCall = api.tx.sudo.sudo(call);

        let attempt = 0;
        while (attempt < MAX_RETRIES) {
            console.log(`Submitting batch ${Math.floor(i / BATCH_SIZE) + 1}, attempt ${attempt + 1}...`);
            try {
                const unsub = await sudoCall.signAndSend(sudo, (result) => {
                    if (result.status.isInBlock) {
                        console.log(`📦 Batch included in block: ${result.status.asInBlock}`);
                        unsub();
                    } else if (result.isError) {
                        console.error('❌ Transaction error reported');
                        unsub();
                    }
                });
                break; // exit retry loop if no exception
            } catch (err) {
                console.error(`⚠️ Error submitting batch: ${err.message}`);
                attempt++;
                if (attempt >= MAX_RETRIES) {
                    throw new Error(`❌ Failed to submit batch after ${MAX_RETRIES} attempts`);
                }
                await delay(RETRY_DELAY_MS);
            }
        }

        await delay(RETRY_DELAY_MS);
    }

    console.log('✅ All checkpoint batches submitted.');
    process.exit(0);
}

main().catch((err) => {
    console.error('❌ Error:', err);
    process.exit(1);
});
