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
 *   onboard --node-url <ws> --version <vX.Y.Z> [--bond <CTC>]
 *       Fund a throwaway stash from sudo, bond, rotate + set keys on the target
 *       node, validate, and set the version identity in one batch.
 *
 *   wait-eras --node-url <ws> --eras <N>
 *       Block until N staking eras have elapsed (so newly-onboarded validators
 *       get elected into the active set naturally, without forcing a new era).
 *
 *   increase-validators --node-url <ws> --additional <N>
 *       Raise the staking validator ceiling via sudo
 *       (staking.increaseValidatorCount) so all onboarded validators fit.
 *
 * All accounts are funded from the genesis sudo key (Alice), so this is only
 * ever meant to run against a throwaway local dev chain.
 */
import { Command } from 'commander';
import { newApi, BN, MICROUNITS_PER_CTC, mnemonicGenerate, ApiPromise, KeyringPair } from '../lib';
import { CcKeyring, initKeyringPair } from '../lib/account/keyring';
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

async function onboard(nodeUrl: string, version: string, bondCtc: number): Promise<void> {
    const { api } = await newApi(nodeUrl);

    // Fresh stash funded from sudo. The mnemonic is throwaway (dev chain only).
    const stashSecret = mnemonicGenerate();
    const stashPair = initKeyringPair(stashSecret);
    const stash: CcKeyring = { type: 'caller', pair: stashPair };
    console.log(`INFO: onboarding ${version} with stash ${stashPair.address}`);

    // Fund well above the minimum validator bond so bonding always succeeds.
    const fundAmount = MICROUNITS_PER_CTC.mul(new BN(bondCtc)).mul(new BN(10));
    await sudoCall(api, api.tx.balances.forceSetBalance(stashPair.address, fundAmount.toString()));

    const bondAmount = MICROUNITS_PER_CTC.mul(new BN(bondCtc));

    // Generate session keys *on the target node* so the keystore holds them.
    const keys = (await api.rpc.author.rotateKeys()).toString();
    console.log(`INFO: ${version} rotated session keys`);

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

    const result = await signSendAndWatchCcKeyring(batch, api, stash);
    if (result.status !== TxStatus.ok) {
        throw new Error(`onboard batch failed for ${version}: ${result.info}`);
    }
    console.log(`DONE: ${version} bonded, set keys, validating, identity=${display}`);
    await api.disconnect();
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

async function waitEras(nodeUrl: string, eras: number): Promise<void> {
    const { api } = await newApi(nodeUrl);
    let eraInfo = await api.derive.session.info();
    let currentEra = eraInfo.currentEra.toNumber();
    const targetEra = currentEra + eras;
    const blockTime = api.consts.babe.expectedBlockTime.toNumber();
    while (currentEra < targetEra) {
        console.log(`INFO: waiting for era ${targetEra}, currently at ${currentEra}`);
        await sleep(blockTime);
        eraInfo = await api.derive.session.info();
        currentEra = eraInfo.currentEra.toNumber();
    }
    console.log(`DONE: reached era ${currentEra}`);
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
        .command('onboard')
        .requiredOption('--node-url <url>', 'WS RPC url of the node to onboard as validator')
        .requiredOption('--version <version>', 'Version string used as the on-chain identity (e.g. v3.66.0)')
        .option('--bond <ctc>', 'Amount of CTC to bond', (v) => parseInt(v, 10), 1000)
        .action(async (opts) => {
            await onboard(opts.nodeUrl, opts.version, opts.bond);
            process.exit(0);
        });

    program
        .command('wait-eras')
        .requiredOption('--node-url <url>', 'WS RPC url of the node to query')
        .requiredOption('--eras <eras>', 'How many eras to wait', (v) => parseInt(v, 10))
        .action(async (opts) => {
            await waitEras(opts.nodeUrl, opts.eras);
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
