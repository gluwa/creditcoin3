#!/usr/bin/env node

/**
 * Submit proof to block-prover precompile
 * 
 * Usage: node submit-proof.js <chainKey> <blockHeight> <txHash> <privateKey> [options]
 * 
 * Options:
 *   --api-url <url>        Proof API server URL (default: http://localhost:3100)
 *   --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)
 *   --source-rpc-url <url> Source chain RPC URL (default: http://localhost:8545)
 *   --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)
 */

const { ethers } = require('ethers');
const fs = require('fs');
const path = require('path');

const fetch = globalThis.fetch || require('node-fetch');

const DEFAULT_PRECOMPILE_ADDRESS = '0x0000000000000000000000000000000000000FD2';
const DEFAULT_API_URL = 'http://localhost:3100';
const DEFAULT_CC3_RPC_URL = 'http://localhost:9944';
const DEFAULT_SOURCE_RPC_URL = 'http://localhost:8545';

const ABI_PATH = path.join(__dirname, '..', 'precompiles', 'metadata', 'abi', 'block_prover.json');
const PRECOMPILE_ABI = JSON.parse(fs.readFileSync(ABI_PATH, 'utf8'));

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        chainKey: null,
        blockHeight: null,
        txHash: null,
        privateKey: null,
        apiUrl: DEFAULT_API_URL,
        cc3RpcUrl: DEFAULT_CC3_RPC_URL,
        sourceRpcUrl: DEFAULT_SOURCE_RPC_URL,
        precompileAddr: DEFAULT_PRECOMPILE_ADDRESS,
    };

    let i = 0;
    while (i < args.length) {
        if (args[i] === '--api-url' && i + 1 < args.length) {
            options.apiUrl = args[++i];
        } else if (args[i] === '--cc3-rpc-url' && i + 1 < args.length) {
            options.cc3RpcUrl = args[++i];
        } else if (args[i] === '--source-rpc-url' && i + 1 < args.length) {
            options.sourceRpcUrl = args[++i];
        } else if (args[i] === '--rpc-url' && i + 1 < args.length) {
            options.cc3RpcUrl = options.sourceRpcUrl = args[++i];
        } else if (args[i] === '--precompile-addr' && i + 1 < args.length) {
            options.precompileAddr = args[++i];
        } else if (!options.chainKey) {
            options.chainKey = args[i];
        } else if (!options.blockHeight) {
            options.blockHeight = args[i];
        } else if (!options.txHash) {
            options.txHash = args[i];
        } else if (!options.privateKey) {
            options.privateKey = args[i];
        }
        i++;
    }

    if (!options.chainKey || !options.blockHeight || !options.txHash || !options.privateKey) {
        console.error('Usage: node submit-proof.js <chainKey> <blockHeight> <txHash> <privateKey> [options]');
        console.error('\nOptions:');
        console.error('  --api-url <url>        Proof API server URL (default: http://localhost:3100)');
        console.error('  --cc3-rpc-url <url>    Creditcoin3 RPC URL (default: http://localhost:9944)');
        console.error('  --source-rpc-url <url> Source chain RPC URL (default: http://localhost:8545)');
        console.error('  --precompile-addr <addr> Precompile address (default: 0x0000000000000000000000000000000000000FD2)');
        process.exit(1);
    }

    return options;
}

async function getTxIndexFromBlock(rpcUrl, blockHeight, txHash) {
    const provider = new ethers.JsonRpcProvider(rpcUrl);
    const block = await provider.getBlock(parseInt(blockHeight), true);

    if (!block?.transactions) {
        throw new Error(`Block ${blockHeight} not found or has no transactions`);
    }

    const normalizedHash = txHash.toLowerCase().startsWith('0x') ? txHash.toLowerCase() : '0x' + txHash.toLowerCase();

    for (let i = 0; i < block.transactions.length; i++) {
        const tx = block.transactions[i];
        if ((typeof tx === 'string' && tx.toLowerCase() === normalizedHash) ||
            (tx.hash && tx.hash.toLowerCase() === normalizedHash)) {
            return i;
        }
    }

    throw new Error(`Transaction ${txHash} not found in block ${blockHeight}`);
}

async function fetchProof(apiUrl, chainKey, blockHeight, txIndex, txHash) {
    let url = `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${txHash}`;
    let response = await fetch(url);

    if (!response.ok) {
        url = `${apiUrl}/api/v1/proof/${chainKey}/${blockHeight}/${txIndex}`;
        response = await fetch(url);
    }

    if (!response.ok) {
        const errorText = await response.text();
        throw new Error(`Failed to fetch proof: ${response.status} ${response.statusText}\n${errorText}`);
    }

    return await response.json();
}

function convertProofFormat(apiProof) {
    if (!apiProof.merkle_proof) {
        throw new Error('Merkle proof is missing from API response');
    }

    return {
        continuityProof: {
            lowerEndpointDigest: apiProof.continuity_proof.lower_endpoint_digest,
            blocks: apiProof.continuity_proof.blocks.map(b => ({
                merkleRoot: b.merkle_root,
                digest: b.digest,
            })),
        },
        merkleProof: {
            root: apiProof.merkle_proof.root,
            siblings: apiProof.merkle_proof.siblings.map(s => ({
                hash: s.hash,
                isLeft: s.is_left,
            })),
        },
    };
}

function decodeRevertReason(iface, revertData) {
    if (!revertData) return null;

    try {
        const reason = iface.parseError(revertData);
        return { type: 'custom', name: reason.name, args: reason.args };
    } catch {
        try {
            if (revertData.startsWith('0x08c379a0')) {
                const decoded = ethers.AbiCoder.defaultAbiCoder().decode(['string'], '0x' + revertData.slice(10));
                return { type: 'error', message: decoded[0] };
            } else if (revertData.startsWith('0x4e487b71')) {
                const decoded = ethers.AbiCoder.defaultAbiCoder().decode(['uint256'], '0x' + revertData.slice(10));
                return { type: 'panic', code: decoded[0] };
            }
        } catch { }
    }
    return null;
}

async function submitToPrecompile(provider, signer, precompileAddr, chainKey, blockHeight, txBytes, merkleProof, continuityProof) {
    const precompile = new ethers.Contract(precompileAddr, PRECOMPILE_ABI, signer);
    const iface = precompile.interface;
    const signerAddress = await signer.getAddress();

    const funcFragment = iface.getFunction('verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,(bytes32,bytes32)[]))');

    const txBytesHex = Buffer.isBuffer(txBytes)
        ? '0x' + txBytes.toString('hex')
        : txBytes.startsWith('0x') ? txBytes : '0x' + txBytes;

    const merkleProofTuple = [merkleProof.root, merkleProof.siblings.map(s => [s.hash, s.isLeft])];
    const continuityProofTuple = [
        continuityProof.lowerEndpointDigest,
        continuityProof.blocks.map(b => [b.merkleRoot, b.digest])
    ];

    const params = [chainKey, blockHeight, txBytesHex, merkleProofTuple, continuityProofTuple];
    const data = iface.encodeFunctionData(funcFragment, params);

    // Simulate call to check for revert reasons
    try {
        await provider.call({ to: precompileAddr, data, from: signerAddress });
    } catch (simError) {
        const revertReason = decodeRevertReason(iface, simError.data || (simError.error?.data));
        if (revertReason) {
            if (revertReason.type === 'error') {
                throw new Error(`Transaction will revert: ${revertReason.message}`);
            } else if (revertReason.type === 'panic') {
                throw new Error(`Transaction will panic: code ${revertReason.code}`);
            } else {
                throw new Error(`Transaction will revert: ${revertReason.name}`);
            }
        }
        throw new Error(`Transaction will revert: ${simError.message}`);
    }

    // Send transaction
    console.log('📤 Sending transaction...');
    const tx = await signer.sendTransaction({
        to: precompileAddr,
        data,
        gasLimit: 5000000,
    });

    console.log(`✅ Transaction submitted: ${tx.hash}`);
    console.log('⏳ Waiting for confirmation...');

    const receipt = await tx.wait();

    if (receipt.status !== 1) {
        const revertReason = decodeRevertReason(iface, receipt.data);
        if (revertReason?.type === 'error') {
            throw new Error(`Transaction reverted: ${revertReason.message}`);
        }
        throw new Error('Transaction reverted');
    }

    // Check for TransactionVerified event
    const event = receipt.logs
        .map(log => {
            try {
                return precompile.interface.parseLog(log);
            } catch {
                return null;
            }
        })
        .find(parsed => parsed?.name === 'TransactionVerified');

    if (event) {
        console.log(`✅ TransactionVerified event:`);
        console.log(`   Chain Key: ${event.args.chainKey}`);
        console.log(`   Height: ${event.args.height}`);
        console.log(`   Transaction Index: ${event.args.transactionIndex}`);
    }

    console.log(`✅ Transaction confirmed in block ${receipt.blockNumber}`);
    console.log(`   Gas used: ${receipt.gasUsed.toString()}`);

    return receipt;
}

async function main() {
    const options = parseArgs();

    console.log('=== Proof Submission ===\n');
    console.log(`Chain Key: ${options.chainKey}`);
    console.log(`Block Height: ${options.blockHeight}`);
    console.log(`Transaction Hash: ${options.txHash}\n`);

    try {
        // Get transaction index
        console.log('Finding transaction index...');
        const txIndex = await getTxIndexFromBlock(options.sourceRpcUrl, options.blockHeight, options.txHash);
        console.log(`✓ Found at index ${txIndex}\n`);

        // Fetch proof from API
        console.log('Fetching proof from API...');
        const apiProof = await fetchProof(options.apiUrl, options.chainKey, options.blockHeight, txIndex, options.txHash);
        console.log(`✓ Proof fetched (cached: ${apiProof.cached})\n`);

        // Get transaction bytes
        if (!apiProof.tx_bytes) {
            throw new Error('Transaction bytes not found in API response');
        }
        const txBytes = Buffer.from(apiProof.tx_bytes.startsWith('0x') ? apiProof.tx_bytes.slice(2) : apiProof.tx_bytes, 'hex');
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
            continuityProof
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
    main().catch(error => {
        console.error('Unhandled error:', error);
        process.exit(1);
    });
}

module.exports = { main, fetchProof, convertProofFormat };
