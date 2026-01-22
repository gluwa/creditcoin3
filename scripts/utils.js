/**
 * Common utilities for Creditcoin3 scripts
 */

const { ethers } = require('ethers');
const fs = require('fs');
const path = require('path');

const fetch = globalThis.fetch || require('node-fetch');

// Constants
const DEFAULT_SOURCE_RPC_URL = process.env.ETH_RPC_URL || 'http://127.0.0.1:8545';
const DEVNET_SOURCE_RPC_URL = 'https://anvil.ccnext-devnet.creditcoin.network';
const DEFAULT_CC3_WS_URL = 'ws://localhost:9944';
const DEFAULT_CC3_HTTP_URL = 'http://localhost:9944';
const DEFAULT_PRIVATE_KEY = '0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80';
const DEFAULT_PRECOMPILE_ADDRESS = '0x0000000000000000000000000000000000000FD2';
const DEFAULT_API_URL = 'http://localhost:3100';
const GAS_BUFFER_MULTIPLIER = 135; // 100% + 35% buffer

// Chain ID to Chain Key mapping (based on chain_spec.rs)
const CHAIN_ID_TO_KEY = {
    1: 1, // Ethereum
    31337: 2, // Anvil1
    11155111: 3, // Sepolia
    31338: 4, // Anvil2
    31339: 5, // Anvil3
    80002: 6, // Polygon amoy testnet
};

const ABI_PATH = path.join(__dirname, '..', 'precompiles', 'metadata', 'abi', 'block_prover.json');

// Chain Key Detection
async function getChainKeyFromChainId(provider) {
    const network = await provider.getNetwork();
    const chainId = Number(network.chainId);

    if (CHAIN_ID_TO_KEY[chainId]) {
        return CHAIN_ID_TO_KEY[chainId];
    }

    throw new Error(`Unknown chain ID: ${chainId}. Please specify --chain-key manually.`);
}

// Transfer Functions
function getRandomEthAddress() {
    return ethers.Wallet.createRandom().address;
}

async function sendTransfer(signer) {
    // Generate a random amount between 0.1 and 1 ETH
    const randomAmount = (Math.random() * (1 - 0.1) + 0.1).toFixed(18);
    const value = ethers.parseEther(randomAmount);

    // Generate a random recipient address
    const recipientAddress = getRandomEthAddress();

    console.log(`📤 Sending ${ethers.formatEther(value)} ETH to ${recipientAddress}...`);

    // Send the transaction
    const tx = await signer.sendTransaction({
        to: recipientAddress,
        value: value,
    });

    console.log(`⏳ Transaction submitted: ${tx.hash}`);
    console.log(`⏳ Waiting for confirmation...`);

    // Wait for the transaction to be mined
    const receipt = await tx.wait();

    console.log(`✅ Transaction mined in block: ${receipt.blockNumber}`);
    console.log(`✅ Transaction hash: ${receipt.hash}`);

    return {
        blockNumber: receipt.blockNumber,
        txHash: receipt.hash,
    };
}

// Attestation Functions
async function waitForAttestation(api, chainKey, blockNumber, maxWaitTime = 300000) {
    console.log(`\n🔍 Waiting for attestation of block ${blockNumber} on chain_key ${chainKey}...`);
    console.log(`   (Max wait time: ${maxWaitTime / 1000}s)`);

    const startTime = Date.now();
    let unsub = null;

    return new Promise((resolve, reject) => {
        // Subscribe to system events
        api.query.system
            .events((events) => {
                for (const record of events) {
                    const { event } = record;

                    // Check for BlockAttested event
                    if (event.section === 'attestation' && event.method === 'BlockAttested') {
                        const [eventChainKey, headerNumber, _digest] = event.data;
                        const attestedChainKey = eventChainKey.toNumber();
                        const attestedBlockNumber = headerNumber.toNumber();

                        console.log(
                            `📢 BlockAttested event: chain_key=${attestedChainKey}, block=${attestedBlockNumber}`,
                        );

                        if (attestedChainKey === chainKey && attestedBlockNumber >= blockNumber) {
                            const elapsed = (Date.now() - startTime) / 1000;
                            console.log(
                                `\n✅ Block ${blockNumber} attested! (attestation for block ${attestedBlockNumber}, elapsed: ${elapsed.toFixed(2)}s)`,
                            );

                            if (unsub) {
                                unsub();
                            }
                            resolve({
                                attestedBlock: attestedBlockNumber,
                                elapsed: elapsed,
                            });
                            return;
                        }
                    }
                }
            })
            .then((unsubscribe) => {
                unsub = unsubscribe;

                // Set timeout
                setTimeout(() => {
                    const elapsed = (Date.now() - startTime) / 1000;
                    if (unsub) {
                        unsub();
                    }
                    reject(new Error(`Timeout waiting for attestation (waited ${elapsed.toFixed(2)}s)`));
                }, maxWaitTime);
            })
            .catch((error) => {
                if (unsub) {
                    unsub();
                }
                reject(error);
            });
    });
}

// Creditcoin3 Block Waiting
async function waitForCreditcoin3Blocks(api, numBlocks = 2) {
    const currentBlock = await api.rpc.chain.getHeader();
    const startBlockNumber = currentBlock.number.toNumber();
    const targetBlockNumber = startBlockNumber + numBlocks;

    console.log(
        `⏳ Waiting for ${numBlocks} Creditcoin3 blocks (current: ${startBlockNumber}, target: ${targetBlockNumber})...`,
    );

    return new Promise((resolve, reject) => {
        let unsub = null;

        api.rpc.chain
            .subscribeNewHeads((header) => {
                const blockNumber = header.number.toNumber();

                if (blockNumber >= targetBlockNumber) {
                    console.log(`✅ Reached block ${blockNumber} (waited ${blockNumber - startBlockNumber} blocks)\n`);
                    if (unsub) {
                        unsub();
                    }
                    resolve(blockNumber);
                }
            })
            .then((unsubscribe) => {
                unsub = unsubscribe;
            })
            .catch((error) => {
                if (unsub) {
                    unsub();
                }
                reject(error);
            });
    });
}

// Proof Functions

/**
 * Fetch proof from the proof generation API
 *
 * @param {string} apiUrl - Base URL of the proof API server
 * @param {number|string} chainKey - Chain key identifier
 * @param {string} txHash - Transaction hash to fetch proof for
 * @param {number} maxRetries - Maximum number of retry attempts (default: 5)
 * @param {number} initialDelay - Initial delay between retries in ms (default: 2000)
 * @param {boolean} verbose - Enable verbose logging (default: false)
 * @returns {Promise<Object>} Proof object containing continuityProof, merkleProof, and txBytes
 *
 * @description
 * Verbose logging (when verbose=true) outputs:
 * - The exact API URL being called
 * - HTTP response status code and status text
 * - Response headers
 * - Success confirmation when response is received
 * - Error response bodies if API calls fail
 *
 * This is useful for debugging API connectivity issues and understanding
 * the proof structure returned by the API.
 */
async function fetchProof(apiUrl, chainKey, txHash, maxRetries = 5, initialDelay = 2000, verbose = false) {
    const url = `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${txHash}`;

    if (verbose) {
        console.log(`API URL: ${url}`);
    }

    let lastError = null;
    for (let attempt = 0; attempt < maxRetries; attempt++) {
        try {
            const response = await fetch(url);

            // Verbose logging: Show HTTP response details
            if (verbose) {
                console.log(`Response status: ${response.status} ${response.statusText}`);
                console.log(`Response headers:`, Object.fromEntries(response.headers.entries()));
            }

            if (response.ok) {
                const jsonData = await response.json();
                // Verbose logging: Confirm successful response reception
                if (verbose) {
                    console.log('API Response received successfully');
                }
                return jsonData;
            }

            // Read error text once (response body can only be consumed once)
            const errorText = await response.text();

            // Verbose logging: Show error response body for debugging
            if (verbose) {
                console.log(`Error response body: ${errorText}`);
            }

            // If it's a 500 error about missing attestation/checkpoint, retry
            if (response.status === 500) {
                try {
                    const errorJson = JSON.parse(errorText);

                    // Check if it's the specific error about missing attestation/checkpoint
                    if (errorJson.message && errorJson.message.includes('No attestation or checkpoint found after')) {
                        if (attempt < maxRetries - 1) {
                            const delay = initialDelay * Math.pow(2, attempt);
                            console.log(
                                `⚠️  Proof API not ready yet (attempt ${attempt + 1}/${maxRetries}), waiting ${delay}ms before retry...`,
                            );
                            await new Promise((resolve) => setTimeout(resolve, delay));
                            lastError = new Error(
                                `Failed to fetch proof: ${response.status} ${response.statusText}\n${errorText}`,
                            );
                            continue;
                        }
                    }
                } catch (parseError) {
                    // If JSON parsing fails, fall through to throw error with raw text
                }
            }

            // For other errors, throw immediately (reuse errorText already read)
            throw new Error(`Failed to fetch proof: ${response.status} ${response.statusText}\n${errorText}`);
        } catch (error) {
            // If it's a network error and we have retries left, retry
            if (attempt < maxRetries - 1 && (error.message.includes('fetch') || error.message.includes('network'))) {
                const delay = initialDelay * Math.pow(2, attempt);
                console.log(
                    `⚠️  Network error (attempt ${attempt + 1}/${maxRetries}), waiting ${delay}ms before retry...`,
                );
                await new Promise((resolve) => setTimeout(resolve, delay));
                lastError = error;
                continue;
            }
            throw error;
        }
    }

    // If we exhausted all retries, throw the last error
    if (lastError) {
        throw lastError;
    }

    throw new Error(`Failed to fetch proof after ${maxRetries} attempts`);
}

function convertProofFormat(apiProof) {
    if (!apiProof.merkleProof) {
        throw new Error('Merkle proof is missing from API response');
    }

    return {
        continuityProof: {
            lowerEndpointDigest: apiProof.continuityProof.lowerEndpointDigest,
            roots: apiProof.continuityProof.roots || apiProof.continuityProof.blocks?.map((b) => b.merkleRoot) || [],
        },
        merkleProof: {
            root: apiProof.merkleProof.root,
            siblings: apiProof.merkleProof.siblings.map((s) => ({
                hash: s.hash,
                isLeft: s.isLeft,
            })),
        },
    };
}

// Precompile Functions
function loadPrecompileABI() {
    return JSON.parse(fs.readFileSync(ABI_PATH, 'utf8'));
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
        } catch {
            // Ignore decode errors
        }
    }
    return null;
}

async function submitToPrecompile(
    provider,
    signer,
    precompileAddr,
    chainKey,
    blockHeight,
    txBytes,
    merkleProof,
    continuityProof,
) {
    const PRECOMPILE_ABI = loadPrecompileABI();
    const precompile = new ethers.Contract(precompileAddr, PRECOMPILE_ABI, signer);
    const iface = precompile.interface;
    const signerAddress = await signer.getAddress();

    const funcFragment = iface.getFunction(
        'verifyAndEmit(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))',
    );

    const txBytesHex = Buffer.isBuffer(txBytes)
        ? '0x' + txBytes.toString('hex')
        : txBytes.startsWith('0x')
            ? txBytes
            : '0x' + txBytes;

    const merkleProofTuple = [merkleProof.root, merkleProof.siblings.map((s) => [s.hash, s.isLeft])];
    const continuityProofTuple = [
        continuityProof.lowerEndpointDigest,
        continuityProof.roots || continuityProof.blocks?.map((b) => b.merkleRoot) || [],
    ];

    const params = [chainKey, blockHeight, txBytesHex, merkleProofTuple, continuityProofTuple];
    const data = iface.encodeFunctionData(funcFragment, params);

    // Simulate call to check for revert reasons
    try {
        await provider.call({ to: precompileAddr, data, from: signerAddress });
    } catch (simError) {
        // Try to extract revert data from various error formats
        let revertData = simError.data || simError.error?.data || simError.reason?.data;

        // If revertData is a string, try to extract it
        if (typeof revertData === 'string' && revertData.startsWith('0x')) {
            // Already in hex format
        } else if (simError.reason) {
            revertData = simError.reason.data;
        }

        const revertReason = decodeRevertReason(iface, revertData);
        if (revertReason) {
            if (revertReason.type === 'error') {
                throw new Error(`Transaction will revert: ${revertReason.message}`);
            } else if (revertReason.type === 'panic') {
                throw new Error(`Transaction will panic: code ${revertReason.code}`);
            } else if (revertReason.type === 'custom') {
                throw new Error(`Transaction will revert: ${revertReason.name}(${revertReason.args?.join(', ') || ''})`);
            } else {
                throw new Error(`Transaction will revert: ${revertReason.name || 'Unknown error'}`);
            }
        }

        // If we couldn't decode, show more debug info
        const errorMsg = simError.message || simError.toString();
        const dataStr = revertData ? ` (data: ${typeof revertData === 'string' ? revertData.substring(0, 100) : JSON.stringify(revertData)})` : '';

        // Log the full error for debugging
        if (process.env.DEBUG) {
            console.error('Full simulation error:', JSON.stringify(simError, null, 2));
        }

        throw new Error(`Transaction will revert: ${errorMsg}${dataStr}`);
    }

    // Estimate gas and add buffer
    console.log('⏳ Estimating gas...');
    let gasLimit;
    try {
        const estimatedGas = await provider.estimateGas({
            to: precompileAddr,
            data,
            from: signerAddress,
        });
        gasLimit = (estimatedGas * BigInt(GAS_BUFFER_MULTIPLIER)) / BigInt(100);
        console.log(`   Estimated gas: ${estimatedGas.toString()}, Gas limit with buffer: ${gasLimit.toString()}`);
    } catch (gasEstimateError) {
        // Gas estimation can fail even when the call would succeed
        // This is a known issue with precompiles - pallet-evm doesn't always
        // properly propagate revert reasons during estimation mode
        // Calculate a reasonable estimate based on continuity proof size (matching Rust logic)
        const continuityBlocks = continuityProof.roots?.length || continuityProof.blocks?.length || 1;
        // Base: 21000 (tx) + ~5000 per continuity block + ~10000 for merkle + overhead
        const calculatedGas = 21000 + continuityBlocks * 5000 + 20000;
        console.warn(`   Gas estimation failed: ${gasEstimateError.toString()}`);
        console.log(
            `   Using calculated gas limit based on proof size: ${calculatedGas} (${continuityBlocks} continuity blocks)`,
        );
        gasLimit = BigInt(calculatedGas);
    }

    // Send transaction
    console.log('📤 Sending transaction...');
    const tx = await signer.sendTransaction({
        to: precompileAddr,
        data,
        gasLimit,
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
        .map((log) => {
            try {
                return precompile.interface.parseLog(log);
            } catch {
                return null;
            }
        })
        .find((parsed) => parsed?.name === 'TransactionVerified');

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

module.exports = {
    // Constants
    DEFAULT_SOURCE_RPC_URL,
    DEVNET_SOURCE_RPC_URL,
    DEFAULT_CC3_WS_URL,
    DEFAULT_CC3_HTTP_URL,
    DEFAULT_PRIVATE_KEY,
    DEFAULT_PRECOMPILE_ADDRESS,
    DEFAULT_API_URL,
    CHAIN_ID_TO_KEY,

    // Functions
    getChainKeyFromChainId,
    getRandomEthAddress,
    sendTransfer,
    waitForAttestation,
    waitForCreditcoin3Blocks,
    fetchProof,
    convertProofFormat,
    loadPrecompileABI,
    decodeRevertReason,
    submitToPrecompile,
};
