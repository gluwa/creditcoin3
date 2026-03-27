#!/usr/bin/env node

/**
 * Chill an attestor on Creditcoin3
 *
 * Usage: node AttestorChill.js --private-key <key> --chain-key <chain> [--attestor <address>] [options]
 *
 * Options:
 *   --private-key <key>    Sr25519 seed phrase or ECDSA private key for signing (required)
 *   --chain-key <chain>    Chain key identifier, e.g. 3 for Sepolia (required)
 *   --attestor <address>   Attestor account address (defaults to signer address)
 *   --url <url>            Creditcoin3 WebSocket RPC URL
 *   --ecdsa                Use ECDSA key type instead of sr25519
 */

const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');

const DEFAULT_URL = 'wss://rpc.ccnext-testnet.creditcoin.network/ws';

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        privateKey: null,
        chainKey: null,
        attestor: null,
        url: DEFAULT_URL,
        ecdsa: false,
    };

    let i = 0;
    while (i < args.length) {
        if (args[i] === '--private-key' && i + 1 < args.length) {
            options.privateKey = args[++i];
        } else if (args[i] === '--chain-key' && i + 1 < args.length) {
            options.chainKey = parseInt(args[++i], 10);
        } else if (args[i] === '--attestor' && i + 1 < args.length) {
            options.attestor = args[++i];
        } else if (args[i] === '--url' && i + 1 < args.length) {
            options.url = args[++i];
        } else if (args[i] === '--ecdsa') {
            options.ecdsa = true;
        } else if (args[i] === '--help' || args[i] === '-h') {
            printUsage();
            process.exit(0);
        } else {
            console.error(`Unknown option: ${args[i]}`);
            printUsage();
            process.exit(1);
        }
        i++;
    }

    if (!options.privateKey || options.chainKey === null) {
        console.error('Error: --private-key and --chain-key are required\n');
        printUsage();
        process.exit(1);
    }

    if (isNaN(options.chainKey)) {
        console.error('Error: --chain-key must be a number');
        process.exit(1);
    }

    return options;
}

function printUsage() {
    console.error('Usage: node AttestorChill.js --private-key <key> --chain-key <chain> [options]');
    console.error('');
    console.error('Options:');
    console.error('  --private-key <key>    Sr25519 seed phrase or ECDSA private key (required)');
    console.error('  --chain-key <chain>    Chain key identifier (required)');
    console.error('  --attestor <address>   Attestor account address (defaults to signer address)');
    console.error(
        '  --url <url>            WebSocket RPC URL (default: wss://rpc.ccnext-testnet.creditcoin.network/ws)',
    );
    console.error('  --ecdsa                Use ECDSA key type instead of sr25519');
    console.error('  -h, --help             Show this help message');
}

function createKeyringPair(privateKey, useEcdsa) {
    const type = useEcdsa ? 'ecdsa' : 'sr25519';
    const keyring = new Keyring({ type });
    return keyring.addFromUri(privateKey);
}

function signSendAndWatch(tx, api, signer) {
    return new Promise((resolve, reject) => {
        let maybeUnsub;

        api.rpc.system
            .accountNextIndex(signer.address)
            .then((nonce) => {
                tx.signAndSend(signer, { nonce }, ({ status, dispatchError }) => {
                    if (status.isFinalized) {
                        if (maybeUnsub) maybeUnsub();
                        resolve({
                            success: true,
                            blockHash: status.asFinalized.toString(),
                        });
                    }

                    if (dispatchError) {
                        if (maybeUnsub) maybeUnsub();

                        if (dispatchError.isModule) {
                            const decoded = api.registry.findMetaError(dispatchError.asModule);
                            reject(new Error(`${decoded.section}.${decoded.name}: ${decoded.docs.join(' ')}`));
                        } else {
                            reject(new Error(dispatchError.toString()));
                        }
                    }
                })
                    .then((unsub) => {
                        maybeUnsub = unsub;
                    })
                    .catch(reject);
            })
            .catch(reject);
    });
}

async function main() {
    const options = parseArgs();

    console.log('=== Attestor Chill ===\n');
    console.log(`RPC URL: ${options.url}`);
    console.log(`Chain Key: ${options.chainKey}`);
    console.log(`Key Type: ${options.ecdsa ? 'ecdsa' : 'sr25519'}\n`);

    console.log('Connecting to chain...');
    const provider = new WsProvider(options.url);
    const api = await ApiPromise.create({ provider });
    await api.isReady;
    console.log('Connected\n');

    const pair = createKeyringPair(options.privateKey, options.ecdsa);
    const signerAddress = pair.address;
    console.log(`Signer Address: ${signerAddress}`);

    const attestorAddress = options.attestor || signerAddress;
    console.log(`Attestor Address: ${attestorAddress}\n`);

    try {
        const attestorEntry = await api.query.attestation.attestors(options.chainKey, attestorAddress);

        if (attestorEntry.isNone) {
            console.error(`Error: No attestor ${attestorAddress} found for chain key ${options.chainKey}`);
            await api.disconnect();
            process.exit(1);
        }

        const attestorData = attestorEntry.unwrap();

        if (attestorData.stash.toString() !== signerAddress) {
            console.error(`Error: Attestor ${attestorAddress} is not owned by signer ${signerAddress}`);
            console.error(`       Attestor stash: ${attestorData.stash.toString()}`);
            await api.disconnect();
            process.exit(1);
        }

        if (attestorData.status.isIdle) {
            console.log('Attestor is already chilled (Idle status)');
            await api.disconnect();
            process.exit(0);
        }

        console.log(`Attestor status: ${attestorData.status.type}`);
        console.log('Submitting chill transaction...\n');

        const tx = api.tx.attestation.chill(options.chainKey, attestorAddress);
        const result = await signSendAndWatch(tx, api, pair);

        console.log(`Transaction finalized at block: ${result.blockHash}`);
        console.log('\nAttestor chilled successfully');
    } catch (error) {
        console.error(`\nError: ${error.message}`);
        if (process.env.DEBUG) {
            console.error(error.stack);
        }
        await api.disconnect();
        process.exit(1);
    }

    await api.disconnect();
    process.exit(0);
}

if (require.main === module) {
    main().catch((error) => {
        console.error('Unhandled error:', error);
        process.exit(1);
    });
}

module.exports = { main };
