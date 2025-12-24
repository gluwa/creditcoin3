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

async function delay(ms) {
    return new Promise((res) => setTimeout(res, ms));
}

async function main() {
    if (IS_DEV) {
        console.log('Running in DEV mode: RETRY_DELAY_MS set to 6000ms');
    }

    // Import configurations from .env file
    const mnemonic = process.env.MNEMONIC;
    if (!mnemonic) {
        throw new Error('MNEMONIC not found in .env file');
    }
    const destinationChain = process.env.DESTINATION_CHAIN;
    if (!destinationChain) {
        throw new Error('DESTINATION_CHAIN not found in .env file');
    }
    const chainKey = process.env.CHAIN_KEY_ON_DESTINATION;
    if (!chainKey) {
        throw new Error('CHAIN_KEY_ON_DESTINATION not found in .env file');
    }

    // Get api and keyring
    const provider = new WsProvider(destinationChain);
    const api = await ApiPromise.create({ provider });

    const keyring = new Keyring({ type: 'sr25519' });
    const sudo = keyring.addFromUri(mnemonic);
    console.log('Sudo address:', sudo.address);

    // Get checkpoints from file
    const rawData = fs.readFileSync('checkpoints.json');
    const parsed = JSON.parse(rawData);

    // Convert to [digest, { block_number }] tuples
    const entries = Object.entries(parsed);

    for (let i = 0; i < entries.length; i += BATCH_SIZE) {
        const batch = entries.slice(i, i + BATCH_SIZE);

        const checkpointVec = batch.map(([digestHex, { block_number }]) => {
            return api.createType('AttestorPrimitivesAttestationCheckpoint', {
                digest: hexToU8a(digestHex),
                // Use bigint to avoid precision loss when block numbers exceed Number.MAX_SAFE_INTEGER
                block_number: BigInt(block_number),
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
