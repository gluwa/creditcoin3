require('dotenv').config(); // Load environment variables from .env
const { ApiPromise, WsProvider } = require('@polkadot/api');
const { u8aToHex } = require('@polkadot/util');
const fs = require('fs');

async function main() {
    // Import configurations from .env file
    const sourceChain = process.env.SOURCE_CHAIN;
    if (!sourceChain) {
        throw new Error('SOURCE_CHAIN not found in .env file');
    }
    const chainKey = process.env.CHAIN_KEY_ON_TARGET;
    if (!chainKey) {
        throw new Error('CHAIN_KEY_ON_TARGET not found in .env file');
    }

    // Connect to a Substrate node
    const provider = new WsProvider(sourceChain); // Replace with your node
    const api = await ApiPromise.create({ provider });
    console.log('Connected to node:', await api.rpc.system.chain());
    const checkpoints = {};

    // Iterate through all checkpoints for chain key in pallet attestation
    const entries = await api.query.attestation.checkpoints.entries(chainKey);

    for (const [key, value] of entries) {
        // Storage key: (chain_key, block_number) => digest
        const blockNumber = key.args[1].toNumber();
        const digestHex = value.toHex();

        checkpoints[digestHex] = {
            block_number: blockNumber,
        };
    }

    // Convert to array and sort by block_number
    const sortedEntries = Object.entries(checkpoints).sort(
        ([_aDigest, aData], [_bDigest, bData]) => {
            return Number(bData.block_number) - Number(aData.block_number);
        }
    );

    // Rebuild into sorted object
    const sortedCheckpoints = Object.fromEntries(sortedEntries);

    fs.writeFileSync('checkpoints.json', JSON.stringify(sortedCheckpoints, null, 2));
    console.log('✅ Checkpoints written to checkpoints.json (sorted by block_number)');

    process.exit(0);
}

main().catch((error) => {
    console.error('Error:', error);
    process.exit(1);
});