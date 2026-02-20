#!/usr/bin/env node

/**
 * Submit proof to block-prover precompile
 *
 * Usage: node SubmitProof.js <chainKey> <txHash> --private-key <key> [options]
 *
 * Options:
 *   --private-key <key>    Private key for signing transactions (required)
 *   --api-url <url>        Proof API server URL (default: http://localhost:3100)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)
 *   --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)
 *   -v, --verbose          Enable verbose logging (shows API response details)
 *
 * Note: The block height is automatically extracted from the API response (headerNumber).
 *
 * Verbose Logging:
 *   When enabled with -v or --verbose, the script will output:
 *   - The exact API URL being called
 *   - HTTP response status and headers
 *   - Complete API response JSON (including continuityProof, merkleProof, txBytes, etc.)
 *   - Error response bodies if API calls fail
 *
 *   This is useful for debugging proof structure issues, comparing API responses,
 *   and understanding the continuity proof format.
 *
 *   Example:
 *     node SubmitProof.js 3 0x... --private-key 0x... -v
 */

const { ethers } = require('ethers');
const {
    DEFAULT_PRECOMPILE_ADDRESS,
    DEFAULT_API_URL,
    DEFAULT_CC3_HTTP_URL,
    fetchProof,
    convertProofFormat,
    submitToPrecompile,
} = require('./utils');

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        chainKey: null,
        txHash: null,
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
        } else if (!options.txHash) {
            options.txHash = args[i];
        }
        i++;
    }

    if (!options.chainKey || !options.txHash || !options.privateKey) {
        console.error('Usage: node SubmitProof.js <chainKey> <txHash> --private-key <key> [options]');
        console.error('\nOptions:');
        console.error('  --private-key <key>    Private key for signing transactions (required)');
        console.error('  --api-url <url>        Proof API server URL (default: http://localhost:3100)');
        console.error('  --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)');
        console.error(
            '  --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)',
        );
        console.error('  -v, --verbose          Enable verbose logging (shows API response details)');
        process.exit(1);
    }

    return options;
}

async function main() {
    const options = parseArgs();

    console.log('=== Proof Submission ===\n');
    console.log(`Chain Key: ${options.chainKey}`);
    console.log(`Transaction Hash: ${options.txHash}\n`);

    try {
        // Fetch proof from API
        console.log('Fetching proof from API...');
        const apiProof = await fetchProof(options.apiUrl, options.chainKey, options.txHash, 5, 2000, options.verbose);

        // Extract block height from API response
        const blockHeight = apiProof.headerNumber;
        if (!blockHeight) {
            throw new Error('Block height (headerNumber) not found in API response');
        }
        console.log(`✓ Proof fetched (cached: ${apiProof.cached})`);
        console.log(`✓ Block height: ${blockHeight}\n`);

        // Log full API response in verbose mode
        // This includes the complete proof structure: continuityProof (with all blocks),
        // merkleProof (with siblings), txBytes, and metadata
        if (options.verbose) {
            console.log('=== API Response (Verbose) ===');
            console.log('Full API response JSON:');
            console.log(JSON.stringify(apiProof, null, 2));
            console.log('');
        }

        // Get transaction bytes
        if (!apiProof.txBytes) {
            throw new Error('Transaction bytes not found in API response');
        }
        const txBytes = Buffer.from(
            apiProof.txBytes.startsWith('0x') ? apiProof.txBytes.slice(2) : apiProof.txBytes,
            'hex',
        );
        console.log(`✓ Transaction bytes: ${txBytes.length} bytes\n`);

        // Convert proof format
        const { continuityProof, merkleProof } = convertProofFormat(apiProof);

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

        // Submit to precompile
        await submitToPrecompile(
            provider,
            signer,
            options.precompileAddr,
            BigInt(options.chainKey),
            BigInt(blockHeight),
            txBytes,
            merkleProof,
            continuityProof,
        );

        console.log('\n✅ Proof submission completed successfully!');
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
