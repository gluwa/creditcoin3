require('dotenv').config(); // Load environment variables from .env
const { ApiPromise, WsProvider } = require('@polkadot/api');
const fs = require('fs');

async function main() {
    // Import configurations from .env file
    const sourceChain = process.env.SOURCE_CHAIN;
    if (!sourceChain) {
        throw new Error('SOURCE_CHAIN not found in .env file');
    }
    const chainKey = process.env.CHAIN_KEY_ON_SOURCE;
    if (!chainKey) {
        throw new Error('CHAIN_KEY_ON_SOURCE not found in .env file');
    }

    // Connect to a Substrate node
    const provider = new WsProvider(sourceChain); // Replace with your node
    const api = await ApiPromise.create({ provider });
    console.log('Connected to node:', await api.rpc.system.chain());

    // Iterate through all checkpoints for chain key in pallet attestation
    const entries = await api.query.attestation.checkpoints.entries(chainKey);

    const rows = entries.map(([key, value]) => {
        // Storage key: (chain_key, block_number) => digest
        return {
            blockNumber: key.args[1].toString(),
            digestHex: value.toHex(),
        };
    });

    // Sort ascending by block_number so ImportCheckpoints.js, which
    // reverses entries before submission, inserts newest-to-oldest.
    rows.sort((a, b) => Number(a.blockNumber) - Number(b.blockNumber));

    const csv = [...rows.map((r) => `${r.blockNumber},${r.digestHex}`)].join('\n') + '\n';

    fs.writeFileSync(`checkpoints_${chainKey}.csv`, csv);
    console.log(`✅ Checkpoints written to checkpoints_${chainKey}.csv (sorted ascending by block_number)`);

    process.exit(0);
}

main().catch((error) => {
    console.error('Error:', error);
    process.exit(1);
});
