#!/usr/bin/env node

/**
 * Submit batch proof to block-prover precompile
 *
 * Usage: node SubmitBatchProof.js <chainKey> --queries <height1:txHash1,height2:txHash2,...> --private-key <key> [options]
 *
 * Options:
 *   --private-key <key>    Private key for signing transactions (required)
 *   --queries <queries>    Comma-separated list of height:txHash pairs (required)
 *   --api-url <url>        Proof API server URL (default: http://localhost:3100)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)
 *   --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)
 *   -v, --verbose          Enable verbose logging
 *
 * Example:
 *   node SubmitBatchProof.js 3 --queries "9986381:0xabc...,9986382:0xdef...,9986385:0x123..." --private-key 0x...
 *
 * The script will:
 *   1. Fetch individual proofs for each query from the API
 *   2. Build a shared continuity proof covering min to max block heights
 *   3. Submit the batch to the precompile's verifyAndEmit function
 */

const { ethers } = require('ethers');
const {
    DEFAULT_PRECOMPILE_ADDRESS,
    DEFAULT_API_URL,
    DEFAULT_CC3_HTTP_URL,
    fetchProof,
    convertProofFormat,
    submitBatchToPrecompile,
    buildSharedContinuityProof,
} = require('./utils');

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        chainKey: null,
        queries: [], // Array of { height, txHash }
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
        } else if (args[i] === '--queries' && i + 1 < args.length) {
            const queriesStr = args[++i];
            options.queries = queriesStr.split(',').map((q) => {
                const [height, txHash] = q.trim().split(':');
                if (!height || !txHash) {
                    console.error(`Invalid query format: "${q}". Expected "height:txHash"`);
                    process.exit(1);
                }
                return { height: parseInt(height, 10), txHash: txHash.trim() };
            });
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

    if (!options.chainKey || options.queries.length === 0 || !options.privateKey) {
        console.error(
            'Usage: node SubmitBatchProof.js <chainKey> --queries <height1:txHash1,height2:txHash2,...> --private-key <key> [options]',
        );
        console.error('\nOptions:');
        console.error('  --private-key <key>    Private key for signing transactions (required)');
        console.error('  --queries <queries>    Comma-separated list of height:txHash pairs (required)');
        console.error('  --api-url <url>        Proof API server URL (default: http://localhost:3100)');
        console.error('  --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)');
        console.error(
            '  --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)',
        );
        console.error('  -v, --verbose          Enable verbose logging');
        console.error('\nExample:');
        console.error(
            '  node SubmitBatchProof.js 3 --queries "9986381:0xabc...,9986382:0xdef..." --private-key 0x...',
        );
        process.exit(1);
    }

    if (options.queries.length > 10) {
        console.error('Error: Maximum 10 queries allowed per batch');
        process.exit(1);
    }

    return options;
}

async function main() {
    const options = parseArgs();

    console.log('=== Batch Proof Submission ===\n');
    console.log(`Chain Key: ${options.chainKey}`);
    console.log(`Number of queries: ${options.queries.length}`);
    console.log('Queries:');
    for (const q of options.queries) {
        console.log(`  - Height: ${q.height}, TxHash: ${q.txHash.substring(0, 20)}...`);
    }
    console.log('');

    try {
        // Fetch proofs for each query
        console.log('Fetching proofs from API...');
        const queryProofs = [];

        for (const query of options.queries) {
            console.log(`  Fetching proof for height ${query.height}...`);
            const apiProof = await fetchProof(
                options.apiUrl,
                options.chainKey,
                query.txHash,
                5,
                2000,
                options.verbose,
            );

            if (!apiProof.tx_bytes) {
                throw new Error(`Transaction bytes not found for height ${query.height}`);
            }

            queryProofs.push({
                height: query.height,
                txHash: query.txHash,
                proof: apiProof,
            });
        }
        console.log(`✓ Fetched ${queryProofs.length} proofs\n`);

        // Build shared continuity proof
        console.log('Building shared continuity proof...');
        const sharedContinuityProof = buildSharedContinuityProof(queryProofs, options.verbose);
        console.log(`✓ Shared continuity proof built (${sharedContinuityProof.roots.length} blocks)\n`);

        // Prepare batch data
        const heights = queryProofs.map((qp) => BigInt(qp.height));
        const txBytesArray = queryProofs.map((qp) => {
            const txBytesStr = qp.proof.tx_bytes;
            return Buffer.from(txBytesStr.startsWith('0x') ? txBytesStr.slice(2) : txBytesStr, 'hex');
        });
        const merkleProofs = queryProofs.map((qp) => {
            const { merkleProof } = convertProofFormat(qp.proof);
            return merkleProof;
        });

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
            sharedContinuityProof,
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
