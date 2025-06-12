require('dotenv').config(); // Load environment variables from .env
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { hexToU8a } = require('@polkadot/util');
const fs = require('fs');

const BATCH_SIZE = 100;

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

        console.log(`Submitting batch ${i / BATCH_SIZE + 1}...`);
        const unsub = await sudoCall.signAndSend(sudo, (result) => {
            if (result.status.isInBlock) {
                console.log(`📦 Batch included in block: ${result.status.asInBlock}`);
                unsub();
            } else if (result.isError) {
                console.error('❌ Transaction failed');
                unsub();
            }
        });

        await new Promise((res) => setTimeout(res, 6000));
    }

    console.log('✅ All checkpoint batches submitted.');
    process.exit(0);
}

main().catch((err) => {
    console.error('❌ Error:', err);
    process.exit(1);
});