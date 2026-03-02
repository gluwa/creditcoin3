#!/usr/bin/env node

/**
 * Submit batch proof to block-prover precompile
 *
 * Usage: node SubmitBatchProof.js <chainKey> --hashes <txHash1,txHash2,...> --private-key <key> [options]
 *
 * Options:
 *   --private-key <key>    Private key for signing transactions (required)
 *   --hashes <hashes>      Comma-separated list of transaction hashes (required)
 *   --api-url <url>        Proof API server URL (default: http://localhost:3100)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)
 *   --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)
 *   -v, --verbose          Enable verbose logging
 *
 * Example:
 *   node SubmitBatchProof.js 3 --hashes "0xabc...,0xdef...,0x123..." --private-key 0x...
 *
 * The script will:
 *   1. Fetch a proof batch for the specified transaction hashes from the API, which includes:
 *      - Individual proofs for each transaction hash
 *      - A continuity proof covering the range of block heights for the included transactions
 *   2. Build a shared continuity proof covering min to max block heights
 *   3. Submit the batch to the precompile's verifyAndEmit function
 */

const { ethers } = require('ethers');
const {
    DEFAULT_PRECOMPILE_ADDRESS,
    DEFAULT_API_URL,
    DEFAULT_CC3_HTTP_URL,
    fetchProofBatch,
    submitBatchToPrecompile,
} = require('./utils');

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        chainKey: null,
        txHashes: [], // Array of txHash
        privateKey: null,
        apiUrl: DEFAULT_API_URL,
        cc3RpcUrl: DEFAULT_CC3_HTTP_URL,
        precompileAddr: DEFAULT_PRECOMPILE_ADDRESS,
        verbose: false,
    };

    let i = 0;
    while (i < args.length) {
        if (args[i] === '--private-key' && i + 1 < args.length) {
            options.privateKey = args[++i];
        } else if (args[i] === '--hashes' && i + 1 < args.length) {
            const txHashesStr = args[++i];
            options.txHashes = txHashesStr.split(',').map((txHash) => txHash.trim());
        } else if (args[i] === '--api-url' && i + 1 < args.length) {
            options.apiUrl = args[++i];
        } else if (args[i] === '--cc3-rpc-url' && i + 1 < args.length) {
            options.cc3RpcUrl = args[++i];
        } else if (args[i] === '--precompile-addr' && i + 1 < args.length) {
            options.precompileAddr = args[++i];
        } else if (args[i] === '-v' || args[i] === '--verbose') {
            options.verbose = true;
        } else if (!options.chainKey) {
            options.chainKey = args[i];
        }
        i++;
    }

    if (!options.chainKey || options.txHashes.length === 0 || !options.privateKey) {
        console.error(
            'Usage: node SubmitBatchProof.js <chainKey> --hashes <txHash1,txHash2,...> --private-key <key> [options]',
        );
        console.error('\nOptions:');
        console.error('  --private-key <key>    Private key for signing transactions (required)');
        console.error('  --hashes <hashes>      Comma-separated list of transaction hashes (required)');
        console.error('  --api-url <url>        Proof API server URL (default: http://localhost:3100)');
        console.error('  --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)');
        console.error(
            '  --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)',
        );
        console.error('  -v, --verbose          Enable verbose logging');
        console.error('\nExample:');
        console.error('  node SubmitBatchProof.js 3 --hashes "0xabc...,0xdef..." --private-key 0x...');
        process.exit(1);
    }

    if (options.txHashes.length > 10) {
        console.error('Error: Maximum 10 hashes allowed per batch');
        process.exit(1);
    }

    return options;
}

async function main() {
    const options = parseArgs();

    console.log('=== Batch Proof Submission ===\n');
    console.log(`Chain Key: ${options.chainKey}`);
    console.log(`Number of hashes: ${options.txHashes.length}`);
    console.log('Hashes:');
    for (const txHash of options.txHashes) {
        console.log(`  - TxHash: ${txHash.substring(0, 20)}...`);
    }
    console.log('');

    try {
        // Fetch proofs for each query
        console.log('Fetching proofs from API...');

        const apiUrl = options.apiUrl;
        const chainKey = options.chainKey;
        const verbose = options.verbose;
        const txHashes = options.txHashes;

        // Fetch batch proofs from API - this will return a structure containing proofs for all requested txHashes,
        // along with a continuity proof covering the range of block heights
        const apiProofs = await fetchProofBatch(apiUrl, chainKey, txHashes, 5, 2000, verbose);

        // Extract batch data for precompile submission
        const heights = [];
        const txBytesArray = [];
        const merkleProofs = [];
        for (const [headerNumber, proofsMap] of Object.entries(apiProofs.merkleProofs)) {
            for (const [_, proofEntry] of Object.entries(proofsMap)) {
                if (!proofEntry.txBytes) {
                    throw new Error(`Transaction bytes not found for height ${headerNumber}`);
                }

                heights.push(BigInt(headerNumber));
                txBytesArray.push(proofEntry.txBytes);
                merkleProofs.push(proofEntry.merkleProof);
            }
        }

        console.log(`✓ Fetched ${merkleProofs.length} proofs\n`);

        console.log('Batch data prepared:');
        console.log(`  Heights: [${heights.join(', ')}]`);
        console.log(`  TX bytes sizes: [${txBytesArray.map((b) => b.length).join(', ')}]`);
        console.log(`  Merkle proofs: ${merkleProofs.length}`);
        console.log('');

        // Connect to Creditcoin3
        console.log('Connecting to Creditcoin3...');
        const provider = new ethers.JsonRpcProvider(options.cc3RpcUrl);
        const signer = new ethers.Wallet(options.privateKey, provider);
        const signerAddress = await signer.getAddress();
        const balance = await provider.getBalance(signerAddress);
        console.log(`✓ Connected as ${signerAddress}`);
        console.log(`✓ Balance: ${ethers.formatEther(balance)} ETH\n`);

        if (balance === 0n) {
            console.warn('⚠️  Warning: Account balance is 0\n');
        }

        // Submit batch to precompile
        await submitBatchToPrecompile(
            provider,
            signer,
            options.precompileAddr,
            BigInt(options.chainKey),
            heights,
            txBytesArray,
            merkleProofs,
            apiProofs.continuityProof,
        );

        console.log('\n✅ Batch proof submission completed successfully!');
    } catch (error) {
        console.error(`\n✗ Error: ${error.message}`);
        if (error.stack && process.env.DEBUG) {
            console.error(error.stack);
        }
        process.exit(1);
    }
}

if (require.main === module) {
    main().catch((error) => {
        console.error('Unhandled error:', error);
        process.exit(1);
    });
}

module.exports = { main };
