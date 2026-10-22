#!/usr/bin/env node

// CI driver for a local Base-shaped OP-Stack source and a real three-attestor quorum.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { ApiPromise, Keyring, WsProvider } = require('@polkadot/api');
const { ethers } = require('ethers');
const { convertProofFormat, DEFAULT_PRECOMPILE_ADDRESS } = require('./utils');

const sourceUrl = process.env.BASE_RPC_URL || 'http://127.0.0.1:8545';
const cc3Url = process.env.CC3_RPC_URL || 'ws://127.0.0.1:9944';
const proofUrl = process.env.PROOF_API_URL || 'http://127.0.0.1:3100';
const archiverUrl = process.env.ARCHIVER_URL || 'http://127.0.0.1:8080';
const logDir = process.env.LOG_DIR;
assert(logDir, 'LOG_DIR is required');
fs.mkdirSync(logDir, { recursive: true });
const manifestPath = path.join(logDir, 'source.json');
const save = (name, value) => fs.writeFileSync(path.join(logDir, name), JSON.stringify(value, null, 2) + '\n');
const readManifest = () => JSON.parse(fs.readFileSync(manifestPath, 'utf8'));
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function rpc(method, params = []) {
    const response = await fetch(sourceUrl, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ jsonrpc: '2.0', id: 1, method, params }),
        signal: AbortSignal.timeout(15_000),
    });
    assert(response.ok, `source RPC HTTP ${response.status}`);
    const body = await response.json();
    assert(!body.error, `${method}: ${JSON.stringify(body.error)}`);
    return body.result;
}

async function seed() {
    assert.equal(Number(await rpc('eth_chainId')), 84532);
    assert.equal(Number(await rpc('eth_blockNumber')), 0, 'start with a fresh, non-mining source');
    const emitter = '0x000000000000000000000000000000000000beef';
    const topic = ethers.id('BaseAttestorE2E(bytes)');
    // Copy calldata to memory and emit it with LOG1. No Solidity toolchain is needed.
    await rpc('anvil_setCode', [emitter, `0x3660006000377f${topic.slice(2)}366000a100`]);
    const depositSenders = ['0x1234567890123456789012345678901234567890', '0x1234567890123456789012345678901234567891'];
    const [sender] = await rpc('eth_accounts');
    const hashes = [];
    for (const [index, type] of [126, 2, 126].entries()) {
        const tx = {
            from: type === 126 ? depositSenders[index / 2] : sender,
            to: emitter,
            type: ethers.toQuantity(type),
            gas: '0x186a0',
            value: '0x1',
            data: ethers.hexlify(Uint8Array.from([index + 1, 0xab, 0xcd])),
        };
        if (type === 126) {
            tx.sourceHash = ethers.zeroPadValue(ethers.toBeHex(index + 1), 32);
            tx.mint = '0xde0b6b3a7640000';
            tx.isSystemTx = false;
        } else {
            tx.maxFeePerGas = '0x77359400';
            tx.maxPriorityFeePerGas = '0x3b9aca00';
        }
        hashes.push(await rpc('eth_sendTransaction', [tx]));
    }
    await rpc('anvil_mine', [1]);
    const block = await rpc('eth_getBlockByNumber', ['0x1', true]);
    const receipts = await rpc('eth_getBlockReceipts', ['0x1']);
    assert.deepEqual(
        block.transactions.map((tx) => tx.hash),
        hashes,
        'mine all three transactions in FIFO order',
    );
    assert.deepEqual(
        block.transactions.map((tx) => Number(tx.type)),
        [126, 2, 126],
    );
    assert.equal(receipts.length, 3);
    let previousGas = 0n;
    for (const [index, receipt] of receipts.entries()) {
        assert.equal(Number(receipt.status), 1);
        assert.equal(Number(receipt.transactionIndex), index);
        assert.equal(receipt.transactionHash, hashes[index]);
        assert.equal(receipt.logs.length, 1);
        assert.equal(receipt.logs[0].topics[0], topic);
        assert.equal(receipt.logs[0].data, block.transactions[index].input);
        assert.equal(BigInt(receipt.gasUsed), BigInt(receipt.cumulativeGasUsed) - previousGas);
        previousGas = BigInt(receipt.cumulativeGasUsed);
        if (index !== 1) {
            assert.equal(Number(receipt.depositReceiptVersion), 1, 'exercise Canyon receipt encoding');
            assert.equal(Number(receipt.depositNonce), 0);
        }
    }
    save('block.json', block);
    save('receipts.json', receipts);
    save('source.json', { chainId: 84532, height: 1, hashes });
    await rpc('anvil_setIntervalMining', [2]);
    console.log('Seeded a Base-shaped block with two deposits and one type-2 transaction, all emitting logs');
}

// Check both the outer dispatch and sudo's inner result, and wait for finality.
async function sudo(api, call) {
    const signer = new Keyring({ type: 'sr25519' }).addFromUri('//Alice');
    return new Promise((resolve, reject) => {
        let unsubscribe;
        const timer = setTimeout(() => finish(new Error('sudo transaction did not finalize within 90s')), 90_000);
        const finish = (error, events) => {
            clearTimeout(timer);
            unsubscribe?.();
            if (error) reject(error);
            else resolve(events);
        };
        api.tx.sudo
            .sudo(call)
            .signAndSend(signer, { nonce: -1 }, ({ dispatchError, events, status }) => {
                if (dispatchError) return finish(new Error(dispatchError.toString()));
                if (status.isInvalid || status.isDropped || status.isUsurped) {
                    return finish(new Error(`sudo transaction ${status.type}`));
                }
                if (!status.isFinalized) return;
                const result = events.find(({ event }) => api.events.sudo.Sudid.is(event))?.event.data[0];
                if (!result || result.isErr) return finish(new Error(`sudo failed: ${result}`));
                finish(null, events);
            })
            .then((unsub) => {
                unsubscribe = unsub;
            })
            .catch(finish);
    });
}

async function withApi(action) {
    const api = await ApiPromise.create({ provider: new WsProvider(cc3Url) });
    try {
        await action(api);
    } finally {
        await api.disconnect();
    }
}

async function register(api) {
    const manifest = readManifest();
    const events = await sudo(
        api,
        api.tx.supportedChains.registerChain(84532, 'Base local CI', 3, 2, 3, null, null, 0, 'V1', 'FixedDelay:2'),
    );
    const registered = events.find(({ event }) => api.events.supportedChains.ChainRegistered.is(event));
    assert(registered, 'ChainRegistered event missing');
    manifest.chainKey = registered.event.data.chainKey.toNumber();
    save('source.json', manifest);
    fs.writeFileSync(
        path.join(logDir, 'proof.yaml'),
        `bind_host: 127.0.0.1\nbind_port: 3100\nchains:\n` +
            `  - chain_key: ${manifest.chainKey}\n    eth_rpc_url: ${sourceUrl}\n` +
            `    eth_chain_family: op-stack\n    archiver_url: http://127.0.0.1:8080\n`,
    );
    console.log(`Registered Base chain ID 84532 as Creditcoin chain key ${manifest.chainKey}`);
}

async function wait(api) {
    const { chainKey, height } = readManifest();
    const deadline = Date.now() + 10 * 60_000;
    while (Date.now() < deadline) {
        const finalized = await api.at(await api.rpc.chain.getFinalizedHead());
        const active = await finalized.query.attestation.activeAttestors(chainKey);
        const checkpoint = await finalized.query.attestation.lastCheckpoint(chainKey);
        const entries = await finalized.query.attestation.attestations.entries(chainKey);
        const records = entries.filter(([, value]) => value.isSome).map(([, value]) => value.unwrap());
        const quorum = records.find(
            (record) =>
                record.attestation.headerNumber.toNumber() > height &&
                new Set(record.attestors.map((account) => account.toString())).size === 3,
        );
        console.log(`Finalized: active=${active.length}, attestations=${records.length}, checkpoint=${checkpoint}`);
        if (active.length === 3 && quorum && checkpoint.isSome && checkpoint.unwrap().blockNumber.toNumber() >= 4) {
            save(
                'attestations.json',
                records.map((record) => record.toJSON()),
            );
            save('checkpoint.json', checkpoint.toJSON());
            console.log('All three attestors contributed to a finalized post-genesis attestation and checkpoint');
            return;
        }
        await sleep(5_000);
    }
    throw new Error('Base attestor quorum/checkpoint did not finalize within 10 minutes');
}

async function verify() {
    const { chainKey, height, hashes } = readManifest();
    const receipts = JSON.parse(fs.readFileSync(path.join(logDir, 'receipts.json'), 'utf8'));
    const provider = new ethers.JsonRpcProvider(cc3Url.replace(/^ws/, 'http'));
    const abi = require('../precompiles/metadata/abi/block_prover.json');
    const contract = new ethers.Contract(DEFAULT_PRECOMPILE_ADDRESS, abi, provider);
    const verifyProof = contract['verify(uint64,uint64,bytes,(bytes32,(bytes32,bool)[]),(bytes32,bytes32[]))'];
    try {
        for (const [index, hash] of hashes.entries()) {
            const deadline = Date.now() + 120_000;
            let proof;
            while (Date.now() < deadline) {
                const response = await fetch(`${proofUrl}/api/v1/proof/${chainKey}/${height}/${index}`, {
                    signal: AbortSignal.timeout(20_000),
                });
                const body = await response.json();
                if (response.ok) {
                    proof = body;
                    break;
                }
                assert([404, 409, 503].includes(response.status), JSON.stringify(body));
                console.log(`Proof not ready: HTTP ${response.status} ${JSON.stringify(body)}`);
                await sleep(3_000);
            }
            assert(proof, `proof API did not serve transaction ${hash} within 120s`);
            save(`proof-${index}.json`, proof);
            assert.equal(proof.chainKey, chainKey);
            assert.equal(proof.headerNumber, height);
            assert.equal(proof.txIndex, index);
            assert.equal(proof.txHash, hash);
            // The proof builder can fall back to the source RPC; explicitly require the
            // archiver's OP-Stack path to produce the same root as well.
            const archived = await fetch(`${archiverUrl}/roots?from=${height}&to=${height}`, {
                signal: AbortSignal.timeout(15_000),
            });
            assert(archived.ok, `archiver has not stored the attested block: HTTP ${archived.status}`);
            const roots = await archived.json();
            assert.equal(roots.length, 1);
            assert.equal(roots[0].merkle_root, proof.merkleProof.root);
            const coder = ethers.AbiCoder.defaultAbiCoder();
            const [type] = coder.decode(['uint8'], proof.txBytes);
            assert.equal(Number(type), index === 1 ? 2 : 126);
            if (type === 126n) {
                const [, chunks] = coder.decode(['uint8', 'bytes[]'], proof.txBytes);
                assert.equal(chunks.length, 3);
                const [nonce] = coder.decode(['uint64'], chunks[0]);
                assert.equal(nonce, BigInt(receipts[index].depositNonce));
                const [status, gasUsed, logs] = coder.decode(
                    ['uint8', 'uint64', '(address,bytes32[],bytes)[]', 'bytes'],
                    chunks[2],
                );
                assert.equal(status, 1n);
                assert.equal(gasUsed, BigInt(receipts[index].gasUsed));
                assert.equal(logs.length, 1);
                assert.equal(logs[0][2], receipts[index].logs[0].data);
            }
            const { merkleProof, continuityProof } = convertProofFormat(proof);
            const args = [chainKey, height, proof.txBytes, merkleProof, continuityProof];
            assert.equal(await verifyProof(...args), true, `precompile rejected transaction ${hash}`);
            const changed = ethers.getBytes(proof.txBytes);
            changed[changed.length - 1] ^= 1;
            args[2] = ethers.hexlify(changed);
            // A modified leaf must fail inclusion, either returning false or reverting.
            try {
                assert.equal(await verifyProof(...args), false, 'precompile accepted a tampered leaf');
            } catch (error) {
                if (error.code !== 'CALL_EXCEPTION') throw error;
            }
            console.log(`Verified API proof and rejected tampering: type=${type}, hash=${hash}`);
        }
    } finally {
        provider.destroy();
    }
}

async function main() {
    switch (process.argv[2]) {
        case 'seed':
            return seed();
        case 'register':
            return withApi(register);
        case 'wait':
            return withApi(wait);
        case 'verify':
            return verify();
        default:
            throw new Error('Usage: LOG_DIR=... node scripts/BaseAttestorE2E.js seed|register|wait|verify');
    }
}

main().catch((error) => {
    console.error(error);
    process.exit(1);
});
