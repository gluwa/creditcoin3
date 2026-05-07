/**
 * Forward-patch attestation checkpoints (runtime extrinsic `forwardPatchCheckpoints`).
 *
 * Recovery strategy (recommended):
 *   1) Anchor + wipe: one tx with `wipeSuffix: true` and a batch containing only the trusted
 *      checkpoint at `--anchor-height`. That deletes every on-chain checkpoint strictly above it
 *      (subject to pallet `MAX_CHECKPOINT_SUFFIX_WIPE_TOTAL`; raise anchor if wipe fails).
 *   2) Rebuild ladder: submit ascending batches for heights > anchor from checkpoints.json.
 *      All batches use `wipeSuffix: false` except the last batch, which uses `wipeSuffix: true`
 *      so nothing remains above your imported tip.
 *
 * Env (same spirit as ImportCheckpoints.js):
 *   MNEMONIC                 – signer (sudo or operator)
 *   DESTINATION_CHAIN        – WS RPC URL
 *   CHAIN_KEY_ON_DESTINATION – chain key u32
 *
 * Optional:
 *   USE_SUDO=0               – sign `forwardPatchCheckpoints` directly (operators origin). Default uses sudo.sudo.
 *
 * CLI:
 *   node ForwardPatchCheckpoints.js --anchor-height 10525000 [checkpoints.json]
 *
 * Default checkpoint file: checkpoints.json (same shape as ImportCheckpoints: { "0xDigest": { "block_number": N } }).
 */
require('dotenv').config();
const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { hexToU8a } = require('@polkadot/util');
const fs = require('fs');
const path = require('path');

const IS_DEV = process.argv.includes('--dev');
const BATCH_SIZE = 100;
const MAX_RETRIES = 10;
const RETRY_DELAY_MS = IS_DEV ? 6000 : 15000;
async function delay(ms) {
    return new Promise((res) => setTimeout(res, ms));
}

function parseArgs(argv) {
    let anchorHeight = null;
    const files = [];
    for (let i = 2; i < argv.length; i++) {
        const a = argv[i];
        if (a === '--anchor-height' && argv[i + 1]) {
            anchorHeight = BigInt(argv[i + 1]);
            i++;
            continue;
        }
        if (!a.startsWith('--')) {
            files.push(a);
        }
    }
    const checkpointPath = files[0] ? files[0] : 'checkpoints.json';
    return { anchorHeight, checkpointPath };
}

function loadSortedEntries(checkpointPath) {
    const raw = fs.readFileSync(checkpointPath);
    const parsed = JSON.parse(raw.toString());
    const entries = Object.entries(parsed).map(([digestHex, meta]) => ({
        digestHex,
        block_number: BigInt(meta.block_number),
    }));
    entries.sort((a, b) => {
        if (a.block_number < b.block_number) return -1;
        if (a.block_number > b.block_number) return 1;
        return 0;
    });
    return entries;
}

function toCheckpointTypes(api, rows) {
    return rows.map(({ digestHex, block_number }) =>
        api.createType('AttestorPrimitivesAttestationCheckpoint', {
            digest: hexToU8a(digestHex),
            block_number,
        }),
    );
}

async function sendBatch(api, signer, chainKey, wipeSuffix, checkpointStructs, useSudo, label) {
    const boundedVec = api.createType(
        'BoundedVec<AttestorPrimitivesAttestationCheckpoint, 100>',
        checkpointStructs,
    );
    const call = api.tx.attestation.forwardPatchCheckpoints(chainKey, wipeSuffix, boundedVec);
    const tx = useSudo ? api.tx.sudo.sudo(call) : call;

    let attempt = 0;
    while (attempt < MAX_RETRIES) {
        console.log(`${label} — submit attempt ${attempt + 1}/${MAX_RETRIES} (wipeSuffix=${wipeSuffix})`);
        try {
            await new Promise((resolve, reject) => {
                let unsub = () => {};
                tx.signAndSend(signer, (result) => {
                    const { status, dispatchError } = result;
                    if (dispatchError) {
                        if (dispatchError.isModule) {
                            const meta = api.registry.findMetaError(dispatchError.asModule);
                            const desc = `${meta.section}.${meta.name}: ${meta.docs.join(' ')}`;
                            unsub();
                            reject(new Error(desc));
                        } else {
                            unsub();
                            reject(new Error(dispatchError.toString()));
                        }
                        return;
                    }
                    if (status?.isFinalized) {
                        console.log(`✅ ${label} finalized in ${status.asFinalized}`);
                        unsub();
                        resolve();
                    }
                })
                    .then((unsubscribe) => {
                        unsub = unsubscribe;
                    })
                    .catch(reject);
            });
            return;
        } catch (err) {
            console.error(`⚠️ ${label}: ${err.message}`);
            attempt++;
            if (attempt >= MAX_RETRIES) {
                throw new Error(`❌ ${label} failed after ${MAX_RETRIES} attempts`);
            }
            await delay(RETRY_DELAY_MS);
        }
    }
}

async function main() {
    const { anchorHeight, checkpointPath } = parseArgs(process.argv);
    if (anchorHeight === null) {
        throw new Error('Missing required flag: --anchor-height <n>');
    }

    const mnemonic = process.env.MNEMONIC;
    if (!mnemonic) {
        throw new Error('MNEMONIC not found in .env file');
    }
    const destinationChain = process.env.DESTINATION_CHAIN;
    if (!destinationChain) {
        throw new Error('DESTINATION_CHAIN not found in .env file');
    }
    const chainKey = Number(process.env.CHAIN_KEY_ON_DESTINATION);
    if (!Number.isFinite(chainKey)) {
        throw new Error('CHAIN_KEY_ON_DESTINATION not found or invalid in .env file');
    }

    const useSudo = process.env.USE_SUDO !== '0' && process.env.USE_SUDO !== 'false';

    const absPath = path.resolve(process.cwd(), checkpointPath);
    console.log(`Checkpoint file: ${absPath}`);
    console.log(`Anchor height (wipe above): ${anchorHeight}`);
    console.log(`USE_SUDO wrapper: ${useSudo}`);

    const entries = loadSortedEntries(absPath);

    const anchorRows = entries.filter((e) => e.block_number === anchorHeight);
    if (anchorRows.length !== 1) {
        throw new Error(
            `Expected exactly one checkpoint entry at anchor height ${anchorHeight}, found ${anchorRows.length}`,
        );
    }

    const ladderRows = entries.filter((e) => e.block_number > anchorHeight);

    const provider = new WsProvider(destinationChain);
    const api = await ApiPromise.create({ provider });
    const keyring = new Keyring({ type: 'sr25519' });
    const signer = keyring.addFromUri(mnemonic);
    console.log('Signer address:', signer.address);

    // 1) Anchor + wipe suffix
    const anchorStructs = toCheckpointTypes(api, anchorRows);
    await sendBatch(
        api,
        signer,
        chainKey,
        true,
        anchorStructs,
        useSudo,
        'Step 1 — anchor + wipe checkpoints above anchor',
    );
    await delay(RETRY_DELAY_MS);

    // 2) Forward-patch ladder
    if (ladderRows.length === 0) {
        console.log('No checkpoints above anchor in file; done after wipe.');
        await api.disconnect();
        process.exit(0);
    }

    const chunkCount = Math.ceil(ladderRows.length / BATCH_SIZE);
    for (let i = 0; i < ladderRows.length; i += BATCH_SIZE) {
        const batchRows = ladderRows.slice(i, i + BATCH_SIZE);
        const chunkIndex = Math.floor(i / BATCH_SIZE);
        const isLast = chunkIndex === chunkCount - 1;
        const wipeSuffix = isLast;
        const structs = toCheckpointTypes(api, batchRows);
        await sendBatch(
            api,
            signer,
            chainKey,
            wipeSuffix,
            structs,
            useSudo,
            `Step 2 — ladder batch ${chunkIndex + 1}/${chunkCount} (${batchRows.length} checkpoints, tip=${batchRows[batchRows.length - 1].block_number})`,
        );
        await delay(RETRY_DELAY_MS);
    }

    console.log('✅ Forward-patch sequence submitted.');
    await api.disconnect();
    process.exit(0);
}

main().catch((err) => {
    console.error('❌ Error:', err);
    process.exit(1);
});
