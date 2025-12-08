#!/usr/bin/env node

/**
 * Transfer funds on source chain and wait for the transaction to be attested on Creditcoin3
 *
 * Usage: node TransferAndWait.js [--chain-key <key>] [--source-rpc-url <url>] [--cc3-rpc-url <url>] [--private-key <key>] [--devnet]
 *
 * Options:
 *   --chain-key <key>      Chain key for the source chain (default: auto-detect from chain ID)
 *   --source-rpc-url <url> Source chain RPC URL (default: http://127.0.0.1:8545)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: ws://localhost:9944)
 *   --private-key <key>    Private key for signing transactions (default: Anvil Account #0)
 *   --devnet               Use devnet provider URL for source chain
 */

const { ethers } = require('ethers');
const { ApiPromise, WsProvider } = require('@polkadot/api');
const {
    DEFAULT_SOURCE_RPC_URL,
    DEVNET_SOURCE_RPC_URL,
    DEFAULT_CC3_WS_URL,
    DEFAULT_PRIVATE_KEY,
    getChainKeyFromChainId,
    sendTransfer,
    waitForAttestation,
} = require('./utils');

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        chainKey: null,
        sourceRpcUrl: DEFAULT_SOURCE_RPC_URL,
        cc3RpcUrl: DEFAULT_CC3_WS_URL,
        privateKey: DEFAULT_PRIVATE_KEY,
        useDevnet: false,
    };

    let i = 0;
    while (i < args.length) {
        if (args[i] === '--chain-key' && i + 1 < args.length) {
            options.chainKey = parseInt(args[++i], 10);
        } else if (args[i] === '--source-rpc-url' && i + 1 < args.length) {
            options.sourceRpcUrl = args[++i];
        } else if (args[i] === '--cc3-rpc-url' && i + 1 < args.length) {
            options.cc3RpcUrl = args[++i];
        } else if (args[i] === '--private-key' && i + 1 < args.length) {
            options.privateKey = args[++i];
        } else if (args[i] === '--devnet') {
            options.useDevnet = true;
            options.sourceRpcUrl = DEVNET_SOURCE_RPC_URL;
        } else if (args[i].startsWith('--')) {
            console.error(`Error: Unknown option: ${args[i]}`);
            console.error('');
            console.error(
                'Usage: node TransferAndWait.js [--chain-key <key>] [--source-rpc-url <url>] [--cc3-rpc-url <url>] [--private-key <key>] [--devnet]',
            );
            process.exit(1);
        }
        i++;
    }

    return options;
}

async function main() {
    const options = parseArgs();

    console.log('=== Transfer and Wait for Attestation ===\n');

    try {
        // Setup source chain provider and signer
        const sourceProvider = new ethers.JsonRpcProvider(options.sourceRpcUrl);
        const signer = new ethers.Wallet(options.privateKey, sourceProvider);
        const signerAddress = await signer.getAddress();

        console.log(`🔗 Source Chain RPC: ${options.sourceRpcUrl}`);
        console.log(`👤 Signer Address: ${signerAddress}`);

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
        const balance = await sourceProvider.getBalance(signerAddress);
        console.log(`💰 Balance: ${ethers.formatEther(balance)} ETH\n`);

        if (balance === 0n) {
            throw new Error('Account balance is 0. Cannot send transfer.');
        }

        // Send transfer
        const { blockNumber, txHash } = await sendTransfer(signer);

        // Connect to Creditcoin3
        console.log(`\n🔗 Connecting to Creditcoin3 at ${options.cc3RpcUrl}...`);
        const provider = new WsProvider(options.cc3RpcUrl);
        const api = await ApiPromise.create({ provider });
        await api.isReady;
        console.log('✅ Connected to Creditcoin3\n');

        // Wait for attestation
        const result = await waitForAttestation(api, chainKey, blockNumber);

        console.log(`\n🎉 Success!`);
        console.log(`   Transaction: ${txHash}`);
        console.log(`   Block: ${blockNumber}`);
        console.log(`   Attested at block: ${result.attestedBlock}`);
        console.log(`   Wait time: ${result.elapsed.toFixed(2)}s`);

        await api.disconnect();
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
