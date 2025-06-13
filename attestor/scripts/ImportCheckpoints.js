require('dotenv').config(); // Load environment variables from .env
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { hexToU8a } = require('@polkadot/util');
const fs = require('fs');

const BATCH_SIZE = 100;
const MAX_RETRIES = 10;
const RETRY_DELAY_MS = 6000;

async function delay(ms) {
    return new Promise((res) => setTimeout(res, ms));
}

async function main() {
    // Import configurations from .env file
    const mnemonic = process.env.MNEMONIC;
    if (!mnemonic) {
        throw new Error('MNEMONIC not found in .env file');
    }
    const targetChain = process.env.TARGET_CHAIN;
    if (!targetChain) {
        throw new Error('TARGET_CHAIN not found in .env file');
    }
    const chainKey = process.env.CHAIN_KEY_ON_TARGET;
    if (!chainKey) {
        throw new Error('CHAIN_KEY not found in .env file');
    }

    // Get api and keyring
    const provider = new WsProvider(targetChain);
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
                block_number: Number(block_number),
            });
        });

        const call = api.tx.attestation.importCheckpoints(chainKey, checkpointVec);
        const sudoCall = api.tx.sudo.sudo(call);

        let attempt = 0;
        while (attempt < MAX_RETRIES) {
            console.log(`Submitting batch ${i / BATCH_SIZE + 1}, attempt ${attempt + 1}...`);
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

        await new Promise((res) => setTimeout(res, 6000));
    }

    console.log('✅ All checkpoint batches submitted.');
    process.exit(0);
}

main().catch((err) => {
    console.error('❌ Error:', err);
    process.exit(1);
});