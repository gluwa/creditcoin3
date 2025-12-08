#!/usr/bin/env node

/**
 * Submit proof to block-prover precompile
 *
 * Usage: node SubmitProof.js <chainKey> <blockHeight> <txHash> --private-key <key> [options]
 *
 * Options:
 *   --private-key <key>    Private key for signing transactions (required)
 *   --api-url <url>        Proof API server URL (default: http://localhost:3100)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)
 *   --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)
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
        blockHeight: null,
        txHash: null,
        privateKey: null,
        apiUrl: DEFAULT_API_URL,
        cc3RpcUrl: DEFAULT_CC3_HTTP_URL,
        precompileAddr: DEFAULT_PRECOMPILE_ADDRESS,
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
        } else if (!options.chainKey) {
            options.chainKey = args[i];
        } else if (!options.blockHeight) {
            options.blockHeight = args[i];
        } else if (!options.txHash) {
            options.txHash = args[i];
        }
        i++;
    }

    if (!options.chainKey || !options.blockHeight || !options.txHash || !options.privateKey) {
        console.error('Usage: node SubmitProof.js <chainKey> <blockHeight> <txHash> --private-key <key> [options]');
        console.error('\nOptions:');
        console.error('  --private-key <key>    Private key for signing transactions (required)');
        console.error('  --api-url <url>        Proof API server URL (default: http://localhost:3100)');
        console.error('  --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)');
        console.error(
            '  --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)',
        );
        process.exit(1);
    }

    return options;
}

async function main() {
    const options = parseArgs();

    console.log('=== Proof Submission ===\n');
    console.log(`Chain Key: ${options.chainKey}`);
    console.log(`Block Height: ${options.blockHeight}`);
    console.log(`Transaction Hash: ${options.txHash}\n`);

    try {
        // Fetch proof from API
        console.log('Fetching proof from API...');
        const apiProof = await fetchProof(options.apiUrl, options.chainKey, options.txHash);
        console.log(`✓ Proof fetched (cached: ${apiProof.cached})\n`);

        // Get transaction bytes
        if (!apiProof.tx_bytes) {
            throw new Error('Transaction bytes not found in API response');
        }
        const txBytes = Buffer.from(
            apiProof.tx_bytes.startsWith('0x') ? apiProof.tx_bytes.slice(2) : apiProof.tx_bytes,
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
            BigInt(options.blockHeight),
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
