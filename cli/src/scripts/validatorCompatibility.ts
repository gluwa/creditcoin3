/**
 * Validator compatibility helper used by the `validator-compatibility` CI
 * workflow.
 *
 * The CC3 dev chain is a BABE + GRANDPA + staking (PoS) network whose genesis
 * only seeds a single authority (Alice). To exercise cross-version validator
 * compatibility we therefore onboard each historical release node as a *new*
 * staked validator at run time rather than relying on the well-known dev
 * authorities.
 *
 * This script exposes the bits that have no direct CLI equivalent:
 *
 *   onboard-all --sudo-url <ws> --nodes-file <path> --stashes-out <path> [--bond <CTC>]
 *       Fund a throwaway stash per node from sudo in one batch, then on every
 *       node in parallel: bond, rotate + set keys on that node, validate, and
 *       set the version identity in one batch.
 *
 *   wait-elected --node-url <ws> --stashes-file <path> [--max-eras <N>]
 *       Block until every onboarded stash is in the active validator set (they
 *       get elected naturally, without forcing a new era).
 *
 *   increase-validators --node-url <ws> --additional <N>
 *       Raise the staking validator ceiling via sudo
 *       (staking.increaseValidatorCount) so all onboarded validators fit.
 *
 * All accounts are funded from the genesis sudo key (Alice), so this is only
 * ever meant to run against a throwaway local dev chain.
 */
import * as fs from 'fs';
import { Command } from 'commander';
import { newApi, BN, MICROUNITS_PER_CTC, mnemonicGenerate, ApiPromise, KeyringPair } from '../lib';
import { initKeyringPair } from '../lib/account/keyring';
import { signSendAndWatchCcKeyring, TxStatus } from '../lib/tx';

function aliceSudo(): KeyringPair {
    return initKeyringPair('//Alice');
}

async function sudoCall(api: ApiPromise, call: any): Promise<void> {
    const sudo = aliceSudo();
    const tx = api.tx.sudo.sudo(call);
    const result = await signSendAndWatchCcKeyring(tx, api, { type: 'caller', pair: sudo });
    if (result.status !== TxStatus.ok) {
        throw new Error(`sudo call failed: ${result.info}`);
    }
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

interface NodeToOnboard {
    tag: string;
    port: number;
}

// `nodes.txt` written by the workflow: one "<tag> <rpc port>" line per historical node.
function readNodesFile(nodesFile: string): NodeToOnboard[] {
    return fs
        .readFileSync(nodesFile, 'utf8')
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0)
        .map((line) => {
            const [tag, port] = line.split(/\s+/);
            return { tag, port: parseInt(port, 10) };
        });
}

async function onboardNode(node: NodeToOnboard, stashPair: KeyringPair, bondAmount: BN): Promise<void> {
    // Node identities read v3.66.0 for the release tag 3.66.0-mainnet.
    const version = `v${node.tag.replace(/-[^-]*$/, '')}`;
    const { api } = await newApi(`ws://127.0.0.1:${node.port}`);
    console.log(`INFO: onboarding ${version} with stash ${stashPair.address}`);

    // The stash was funded through Alice. Wait until this node has imported that block,
    // otherwise the bond below is checked against an empty account.
    const fundedBy = Date.now() + 120_000;
    while ((await api.query.system.account(stashPair.address)).data.free.isZero()) {
        if (Date.now() > fundedBy) {
            throw new Error(`${version}: the stash funding never reached this node`);
        }
        await sleep(1_000);
    }

    // Generate session keys *on the target node* so the keystore holds them.
    const keys = (await api.rpc.author.rotateKeys()).toString();

    // Build a legacy IdentityInfo whose display name is the version string,
    // so each validator is trivially identifiable on-chain. polkadot.js fills
    // the omitted fields with their `None` variant automatically.
    const display = version.length > 32 ? version.slice(0, 32) : version;
    const identityInfo = {
        display: { raw: display },
    };

    const batch = api.tx.utility.batchAll([
        api.tx.staking.bond(bondAmount.toString(), 'Staked'),
        api.tx.session.setKeys(keys, ''),
        api.tx.staking.validate({ commission: 0, blocked: false }),
        api.tx.identity.setIdentity(identityInfo),
    ]);

    const result = await signSendAndWatchCcKeyring(batch, api, { type: 'caller', pair: stashPair });
    if (result.status !== TxStatus.ok) {
        throw new Error(`onboard batch failed for ${version}: ${result.info}`);
    }
    console.log(`DONE: ${version} bonded, set keys, validating, identity=${display}`);
    await api.disconnect();
}

// Onboard every historical node at once: one sudo batch funds a fresh stash per node, then
// each stash bonds, sets keys and validates through its own node in parallel. One node at
// a time cost ~30 s each, two finalized transactions apiece. The stash addresses are
// written to `stashesFile` for `wait-elected`.
async function onboardAll(sudoUrl: string, nodesFile: string, bondCtc: number, stashesFile: string): Promise<void> {
    const nodes = readNodesFile(nodesFile);
    // Connect first: it initialises the crypto WASM that key generation needs.
    const { api } = await newApi(sudoUrl);
    // Fresh stashes, throwaway mnemonics (dev chain only).
    const stashes = nodes.map(() => initKeyringPair(mnemonicGenerate()));

    // Fund well above the minimum validator bond so bonding always succeeds.
    const fundAmount = MICROUNITS_PER_CTC.mul(new BN(bondCtc)).mul(new BN(10));
    await sudoCall(
        api,
        api.tx.utility.batchAll(
            stashes.map((stash) => api.tx.balances.forceSetBalance(stash.address, fundAmount.toString())),
        ),
    );
    console.log(`INFO: funded ${stashes.length} stashes`);
    await api.disconnect();

    const bondAmount = MICROUNITS_PER_CTC.mul(new BN(bondCtc));
    await Promise.all(nodes.map((node, i) => onboardNode(node, stashes[i], bondAmount)));

    fs.writeFileSync(stashesFile, stashes.map((stash) => stash.address).join('\n') + '\n');
}

// Block until every onboarded stash is in the active session validator set, so the
// historical nodes are authoring. Validators that start validating mid-era are elected
// for the next era or the one after, so `maxEras` bounds the wait.
async function waitElected(nodeUrl: string, stashesFile: string, maxEras: number): Promise<void> {
    const stashes = fs
        .readFileSync(stashesFile, 'utf8')
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0);
    const { api } = await newApi(nodeUrl);
    const startEra = (await api.derive.session.info()).currentEra.toNumber();
    const blockTime = api.consts.babe.expectedBlockTime.toNumber();

    for (;;) {
        const active = new Set((await api.query.session.validators()).map((v) => v.toString()));
        const missing = stashes.filter((stash) => !active.has(stash));
        const currentEra = (await api.derive.session.info()).currentEra.toNumber();
        if (missing.length === 0) {
            console.log(`DONE: all ${stashes.length} onboarded validators active in era ${currentEra}`);
            break;
        }
        if (currentEra - startEra > maxEras) {
            throw new Error(
                `${missing.length} of ${stashes.length} onboarded validators still not active after ${maxEras} eras: ${missing.join(', ')}`,
            );
        }
        console.log(`INFO: era ${currentEra}: ${stashes.length - missing.length}/${stashes.length} validators active`);
        await sleep(blockTime);
    }
    await api.disconnect();
}

async function increaseValidators(nodeUrl: string, additional: number): Promise<void> {
    if (!Number.isInteger(additional) || additional <= 0) {
        throw new Error('--additional must be a positive integer');
    }
    const { api } = await newApi(nodeUrl);

    const current = (await api.query.staking.validatorCount()).toNumber();
    console.log(`INFO: staking.validatorCount = ${current}, adding ${additional}...`);

    await sudoCall(api, api.tx.staking.increaseValidatorCount(additional));

    const updated = (await api.query.staking.validatorCount()).toNumber();
    console.log(`DONE: staking.validatorCount = ${updated}`);
    await api.disconnect();
}

async function main(): Promise<void> {
    const program = new Command();
    program.description('Validator compatibility CI helper (dev chain only)');

    program
        .command('onboard-all')
        .requiredOption('--sudo-url <url>', 'WS RPC url of the node to send the sudo funding batch to')
        .requiredOption('--nodes-file <path>', 'File with one "<release tag> <rpc port>" line per node to onboard')
        .requiredOption('--stashes-out <path>', 'Where to write the onboarded stash addresses')
        .option('--bond <ctc>', 'Amount of CTC to bond', (v) => parseInt(v, 10), 1000)
        .action(async (opts) => {
            await onboardAll(opts.sudoUrl, opts.nodesFile, opts.bond, opts.stashesOut);
            process.exit(0);
        });

    program
        .command('wait-elected')
        .requiredOption('--node-url <url>', 'WS RPC url of the node to query')
        .requiredOption('--stashes-file <path>', 'Stash addresses written by onboard-all')
        .option('--max-eras <eras>', 'Fail if not all are active after this many eras', (v) => parseInt(v, 10), 3)
        .action(async (opts) => {
            await waitElected(opts.nodeUrl, opts.stashesFile, opts.maxEras);
            process.exit(0);
        });

    program
        .command('increase-validators')
        .requiredOption('--node-url <url>', 'WS RPC url of the node to configure')
        .requiredOption('--additional <additional>', 'How many validator slots to add', (v) => parseInt(v, 10))
        .action(async (opts) => {
            await increaseValidators(opts.nodeUrl, opts.additional);
            process.exit(0);
        });

    await program.parseAsync(process.argv);
}

main().catch((err) => {
    console.error(err);
    process.exit(1);
});
