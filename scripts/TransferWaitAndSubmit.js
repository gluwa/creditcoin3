#!/usr/bin/env node

/**
 * Transfer funds on source chain, wait for attestation, and submit proof to precompile
 *
 * Usage: node TransferWaitAndSubmit.js [options]
 *
 * Options:
 *   --chain-key <key>      Chain key for the source chain (default: auto-detect from chain ID)
 *   --source-rpc-url <url> Source chain RPC URL (default: http://127.0.0.1:8545)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL for both WS and HTTP (default: ws://localhost:9944)
 *   --cc3-ws-url <url>     Creditcoin3 WebSocket RPC URL (default: ws://localhost:9944)
 *   --cc3-http-url <url>   Creditcoin3 HTTP RPC URL (default: http://localhost:9944)
 *   --private-key <key>    Private key for signing source chain transactions (default: Anvil Account #0)
 *   --cc3-private-key <key> Private key for signing Creditcoin3 transactions (default: same as --private-key)
 *   --api-url <url>        Proof API server URL (default: http://localhost:3100)
 *   --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)
 *   --devnet               Use devnet provider URL for source chain
 */

const { ethers } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const {
    DEFAULT_SOURCE_RPC_URL,
    DEVNET_SOURCE_RPC_URL,
    DEFAULT_CC3_WS_URL,
    DEFAULT_CC3_HTTP_URL,
    DEFAULT_PRIVATE_KEY,
    DEFAULT_PRECOMPILE_ADDRESS,
    DEFAULT_API_URL,
    getChainKeyFromChainId,
    sendTransfer,
    waitForAttestation,
    waitForCreditcoin3Blocks,
    fetchProof,
    convertProofFormat,
    submitToPrecompile,
} = require('./utils');

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        chainKey: null,
        sourceRpcUrl: DEFAULT_SOURCE_RPC_URL,
        cc3WsUrl: DEFAULT_CC3_WS_URL,
        cc3HttpUrl: DEFAULT_CC3_HTTP_URL,
        sourcePrivateKey: DEFAULT_PRIVATE_KEY,
        cc3PrivateKey: null, // Will default to sourcePrivateKey if not provided
        apiUrl: DEFAULT_API_URL,
        precompileAddr: DEFAULT_PRECOMPILE_ADDRESS,
        useDevnet: false,
    };

    let i = 0;
    while (i < args.length) {
        if (args[i] === '--chain-key' && i + 1 < args.length) {
            options.chainKey = parseInt(args[++i], 10);
        } else if (args[i] === '--source-rpc-url' && i + 1 < args.length) {
            options.sourceRpcUrl = args[++i];
        } else if (args[i] === '--cc3-rpc-url' && i + 1 < args.length) {
            // If only one CC3 URL is provided, use it for both WS and HTTP
            const url = args[++i];
            if (url.startsWith('ws://') || url.startsWith('wss://')) {
                options.cc3WsUrl = url;
                options.cc3HttpUrl = url.replace(/^ws/, 'http');
            } else if (url.startsWith('http://') || url.startsWith('https://')) {
                options.cc3HttpUrl = url;
                options.cc3WsUrl = url.replace(/^http/, 'ws');
            } else {
                // Assume ws:// if no protocol
                options.cc3WsUrl = `ws://${url}`;
                options.cc3HttpUrl = `http://${url}`;
            }
        } else if (args[i] === '--cc3-ws-url' && i + 1 < args.length) {
            options.cc3WsUrl = args[++i];
        } else if (args[i] === '--cc3-http-url' && i + 1 < args.length) {
            options.cc3HttpUrl = args[++i];
        } else if (args[i] === '--private-key' && i + 1 < args.length) {
            options.sourcePrivateKey = args[++i];
        } else if (args[i] === '--cc3-private-key' && i + 1 < args.length) {
            options.cc3PrivateKey = args[++i];
        } else if (args[i] === '--api-url' && i + 1 < args.length) {
            options.apiUrl = args[++i];
        } else if (args[i] === '--precompile-addr' && i + 1 < args.length) {
            options.precompileAddr = args[++i];
        } else if (args[i] === '--devnet') {
            options.useDevnet = true;
            options.sourceRpcUrl = DEVNET_SOURCE_RPC_URL;
        } else if (args[i].startsWith('--')) {
            console.error(`Error: Unknown option: ${args[i]}`);
            console.error('');
            console.error('Usage: node TransferWaitAndSubmit.js [options]');
            console.error('\nOptions:');
            console.error('  --chain-key <key>      Chain key for the source chain (default: auto-detect)');
            console.error('  --source-rpc-url <url> Source chain RPC URL (default: http://127.0.0.1:8545)');
            console.error(
                '  --cc3-rpc-url <url>    Creditcoin3 RPC URL for both WS and HTTP (default: ws://localhost:9944)',
            );
            console.error('  --cc3-ws-url <url>     Creditcoin3 WebSocket RPC URL (default: ws://localhost:9944)');
            console.error('  --cc3-http-url <url>   Creditcoin3 HTTP RPC URL (default: http://localhost:9944)');
            console.error(
                '  --private-key <key>    Private key for signing source chain transactions (default: Anvil Account #0)',
            );
            console.error(
                '  --cc3-private-key <key> Private key for signing Creditcoin3 transactions (default: same as --private-key)',
            );
            console.error('  --api-url <url>        Proof API server URL (default: http://localhost:3100)');
            console.error(
                '  --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)',
            );
            console.error('  --devnet               Use devnet provider URL for source chain');
            process.exit(1);
        }
        i++;
    }

    // Default cc3PrivateKey to sourcePrivateKey if not provided
    if (!options.cc3PrivateKey) {
        options.cc3PrivateKey = options.sourcePrivateKey;
    }

    return options;
}

async function main() {
    const options = parseArgs();

    console.log('=== Transfer, Wait for Attestation, and Submit Proof ===\n');

    try {
        // Setup source chain provider and signer
        const sourceProvider = new ethers.JsonRpcProvider(options.sourceRpcUrl);
        const sourceSigner = new ethers.Wallet(options.sourcePrivateKey, sourceProvider);
        const sourceSignerAddress = await sourceSigner.getAddress();

        console.log(`🔗 Source Chain RPC: ${options.sourceRpcUrl}`);
        console.log(`👤 Source Chain Signer Address: ${sourceSignerAddress}`);

        // Get or detect chain key
        let chainKey = options.chainKey;
        if (!chainKey) {
            console.log('🔍 Auto-detecting chain key from chain ID...');
            chainKey = await getChainKeyFromChainId(sourceProvider);
            console.log(`✅ Detected chain key: ${chainKey}`);
        } else {
            console.log(`✅ Using chain key: ${chainKey}`);
        }

        // Check balance
        const sourceBalance = await sourceProvider.getBalance(sourceSignerAddress);
        console.log(`💰 Source Chain Balance: ${ethers.formatEther(sourceBalance)} ETH\n`);

        if (sourceBalance === 0n) {
            throw new Error('Source chain account balance is 0. Cannot send transfer.');
        }

        // Step 1: Send transfer
        console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
        console.log('STEP 1: Send Transfer');
        console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n');
        const { blockNumber, txHash } = await sendTransfer(sourceSigner);

        // Step 2: Wait for attestation
        console.log('\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
        console.log('STEP 2: Wait for Attestation');
        console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
        console.log(`\n🔗 Connecting to Creditcoin3 at ${options.cc3WsUrl}...`);
        const wsProvider = new WsProvider(options.cc3WsUrl);
        const api = await ApiPromise.create({ provider: wsProvider });
        await api.isReady;
        console.log('✅ Connected to Creditcoin3\n');

        const attestationResult = await waitForAttestation(api, chainKey, blockNumber);

        // Wait for at least 2 Creditcoin3 blocks to ensure attestation is indexed
        await waitForCreditcoin3Blocks(api, 2);

        await api.disconnect();

        // Step 3: Fetch proof and submit
        console.log('\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
        console.log('STEP 3: Fetch Proof and Submit');
        console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n');

        console.log('Fetching proof from API...');
        const apiProof = await fetchProof(options.apiUrl, chainKey, txHash);
        console.log(`✓ Proof fetched (cached: ${apiProof.cached})\n`);

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

        // Connect to Creditcoin3 via HTTP for transaction submission
        console.log(`🔗 Connecting to Creditcoin3 at ${options.cc3HttpUrl}...`);
        const cc3Provider = new ethers.JsonRpcProvider(options.cc3HttpUrl);
        const cc3Signer = new ethers.Wallet(options.cc3PrivateKey, cc3Provider);
        const cc3SignerAddress = await cc3Signer.getAddress();
        const cc3Balance = await cc3Provider.getBalance(cc3SignerAddress);
        console.log(`✓ Connected as ${cc3SignerAddress}`);
        console.log(`✓ Creditcoin3 Balance: ${ethers.formatEther(cc3Balance)} ETH\n`);

        if (cc3Balance === 0n) {
            console.warn('⚠️  Warning: Creditcoin3 account balance is 0\n');
        }

        // Submit to precompile
        await submitToPrecompile(
            cc3Provider,
            cc3Signer,
            options.precompileAddr,
            BigInt(chainKey),
            BigInt(blockNumber),
            txBytes,
            merkleProof,
            continuityProof,
        );

        console.log('\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
        console.log('🎉 SUCCESS! All steps completed!');
        console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
        console.log(`   Transaction: ${txHash}`);
        console.log(`   Block: ${blockNumber}`);
        console.log(`   Attested at block: ${attestationResult.attestedBlock}`);
        console.log(`   Attestation wait time: ${attestationResult.elapsed.toFixed(2)}s`);
        console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n');

        process.exit(0);
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
