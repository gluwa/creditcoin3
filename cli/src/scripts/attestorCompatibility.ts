/**
 * Attestor compatibility helper used by the `attestor-compatibility` CI
 * workflow.
 *
 * `attestor_zombienet` generates a random mnemonic per attestor internally and
 * only logs the resulting account id, so we cannot sign transactions on behalf
 * of those accounts. To make each historical attestor identifiable in the
 * Polkadot.js portal we instead supply our own mnemonics via the zombienet
 * `--seeds-file` flag, remember the version -> mnemonic mapping, and then set
 * an on-chain identity (display name = version string) from each account.
 *
 * Subcommands:
 *
 *   gen-seeds --versions <csv> --out-dir <dir>
 *       For every version, generate a fresh bip39 mnemonic and write:
 *         <out-dir>/seeds-<version>.txt   (single line, consumed by zombienet)
 *         <out-dir>/identity-map.tsv      (version<TAB>mnemonic<TAB>address)
 *       The mnemonic is throwaway (dev chain only).
 *
 *   set-identity --node-url <ws> --map <identity-map.tsv>
 *       For each row, sign identity.setIdentity({ display: version }) from the
 *       attestor account derived from that mnemonic, so the version is visible
 *       in the Polkadot.js portal.
 *
 * The account derivation here (sr25519, default derivation, no path) matches
 * how the attestor derives its account from the same mnemonic (subxt_signer
 * sr25519 Keypair::from_uri), so the identity is set on the exact account the
 * attestor registered with.
 */
import * as fs from 'fs';
import * as path from 'path';
import { Command } from 'commander';
import { cryptoWaitReady } from '@polkadot/util-crypto';
import { newApi, mnemonicGenerate } from '../lib';
import { CcKeyring, initKeyringPair } from '../lib/account/keyring';
import { signSendAndWatchCcKeyring, TxStatus } from '../lib/tx';

interface IdentityRow {
    version: string;
    mnemonic: string;
    address: string;
}

async function genSeeds(versionsCsv: string, outDir: string): Promise<void> {
    await cryptoWaitReady();
    const versions = versionsCsv
        .split(',')
        .map((v) => v.trim())
        .filter((v) => v.length > 0);

    if (versions.length === 0) {
        throw new Error('no versions provided');
    }

    fs.mkdirSync(outDir, { recursive: true });

    const rows: IdentityRow[] = [];
    for (const version of versions) {
        const mnemonic = mnemonicGenerate();
        const pair = initKeyringPair(mnemonic);
        rows.push({ version, mnemonic, address: pair.address });

        const seedsFile = path.join(outDir, `seeds-${version}.txt`);
        fs.writeFileSync(seedsFile, `${mnemonic}\n`);
        console.log(`INFO: ${version} -> ${pair.address} (seeds: ${seedsFile})`);
    }

    const mapFile = path.join(outDir, 'identity-map.tsv');
    const tsv = rows.map((r) => `${r.version}\t${r.mnemonic}\t${r.address}`).join('\n') + '\n';
    fs.writeFileSync(mapFile, tsv);
    console.log(`DONE: wrote ${rows.length} seed file(s) and ${mapFile}`);
}

const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

// Poll attestation.lastCheckpoint(chainKey) until at least `target` distinct
// checkpoints have been finalized. A finalized checkpoint means a full window
// of attestations (DefaultAttestationsPerCheckpoint, currently 10) was produced
// and agreed on, so it proves the attestor set is live and the pipeline works.
//
// Cadence note: with fast-runtime the source-chain (Anvil) block time is ~6s,
// the attestation interval is 10 blocks (~60s per attestation) and 10
// attestations form one checkpoint, so a single checkpoint takes ~10 min.
// The CI soak targets 5 checkpoints (~50 min) so late-starting historical
// attestors participate in many rounds, not just one. `--timeout-min` bounds
// the wait.
async function waitCheckpoints(nodeUrl: string, chainKey: number, target: number, timeoutMin: number): Promise<void> {
    const MAX_WAIT_MS = timeoutMin * 60 * 1000;
    const POLL_MS = 10_000;

    const { api } = await newApi(nodeUrl);
    const seen = new Set<string>();
    const deadline = Date.now() + MAX_WAIT_MS;

    while (Date.now() < deadline) {
        const last = await api.query.attestation.lastCheckpoint(chainKey);
        if ((last as any).isSome) {
            const digest = (last as any).unwrap().digest.toHex();
            if (!seen.has(digest)) {
                seen.add(digest);
                console.log(`INFO: checkpoint ${seen.size} finalized (digest=${digest})`);
            }
        }
        if (seen.size >= target) {
            console.log(`OK: ${seen.size}/${target} checkpoint(s) confirmed – attestor set is live`);
            await api.disconnect();
            return;
        }
        await sleep(POLL_MS);
    }

    await api.disconnect();
    throw new Error(
        `only ${seen.size}/${target} checkpoint(s) finalized in ${timeoutMin} min – ` +
            `attestors are not producing/agreeing on attestations fast enough ` +
            `(expected ~10 min per checkpoint under fast-runtime)`,
    );
}

// Reads attestation.activeAttestors(chainKey) and all attestation.attestations(
// chainKey, *) entries. Verifies that every registered AccountId appears in at
// least one SignedAttestation.attestors list, confirming their BLS-signed
// contribution reached the chain.
//
// NOTE: if the quorum threshold is < 100% it is possible for some attestors to
// be "compatible" (healthy, participating in P2P gossip) yet not appear in every
// on-chain record. We therefore check for "at least one appearance across all
// records" rather than "present in every record".
async function verifyAttestations(nodeUrl: string, chainKey: number): Promise<void> {
    const { api } = await newApi(nodeUrl);

    const activeVec = await api.query.attestation.activeAttestors(chainKey);
    const attestorSet = new Set<string>((activeVec as any).map((a: any) => a.toString()));
    console.log(`Active attestors on chain-key ${chainKey}: ${attestorSet.size}`);

    const entries = await api.query.attestation.attestations.entries(chainKey);
    const participated = new Set<string>();
    for (const [, value] of entries) {
        if ((value as any).isSome) {
            for (const acc of (value as any).unwrap().attestors) {
                participated.add(acc.toString());
            }
        }
    }
    console.log(`Attestors found in on-chain attestation records: ${participated.size}`);

    const missing = [...attestorSet].filter((a) => !participated.has(a));
    await api.disconnect();

    if (missing.length > 0) {
        throw new Error(`attestors not found in any on-chain record: ${missing.join(', ')}`);
    }
    console.log('OK: all active attestors appear in at least one on-chain attestation record');
}

async function setIdentity(nodeUrl: string, mapFile: string): Promise<void> {
    const contents = fs.readFileSync(mapFile, 'utf8');
    const rows: IdentityRow[] = contents
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line.length > 0)
        .map((line) => {
            const [version, mnemonic, address] = line.split('\t');
            return { version, mnemonic, address };
        });

    if (rows.length === 0) {
        throw new Error(`no rows in ${mapFile}`);
    }

    await cryptoWaitReady();
    const { api } = await newApi(nodeUrl);

    let failures = 0;
    for (const row of rows) {
        const pair = initKeyringPair(row.mnemonic);
        const signer: CcKeyring = { type: 'caller', pair };

        // Legacy IdentityInfo whose display name is the version string. polkadot.js
        // fills the omitted fields with their `None` variant automatically.
        const display = row.version.length > 32 ? row.version.slice(0, 32) : row.version;
        const identityInfo = { display: { raw: display } };

        const tx = api.tx.identity.setIdentity(identityInfo);
        const result = await signSendAndWatchCcKeyring(tx, api, signer);
        if (result.status !== TxStatus.ok) {
            console.error(`ERROR: setIdentity failed for ${row.version} (${pair.address}): ${result.info}`);
            failures += 1;
        } else {
            console.log(`OK: ${row.version} identity set on ${pair.address} (display=${display})`);
        }
    }

    await api.disconnect();

    if (failures > 0) {
        throw new Error(`${failures} identity assignment(s) failed`);
    }
    console.log(`DONE: set identity for ${rows.length} attestor(s)`);
}

async function main(): Promise<void> {
    const program = new Command();

    program
        .command('gen-seeds')
        .description('Generate one mnemonic per version and write zombienet seeds files + identity map')
        .requiredOption('--versions <csv>', 'Comma-separated version tags (e.g. 3.125.0-mainnet,3.128.0-mainnet)')
        .requiredOption('--out-dir <dir>', 'Directory to write seeds-<version>.txt and identity-map.tsv')
        .action(async (opts) => {
            await genSeeds(opts.versions, opts.outDir);
        });

    program
        .command('set-identity')
        .description('Set on-chain identity (display = version) for each attestor account in the identity map')
        .requiredOption('--node-url <ws>', 'CC3 node WS url')
        .requiredOption('--map <file>', 'Path to identity-map.tsv produced by gen-seeds')
        .action(async (opts) => {
            await setIdentity(opts.nodeUrl, opts.map);
        });

    program
        .command('wait-checkpoints')
        .description('Block until N distinct attestation checkpoints have finalized for a chain key')
        .requiredOption('--node-url <ws>', 'CC3 node WS url')
        .option('--chain-key <n>', 'Source chain key', '2')
        .option('--target <n>', 'Number of distinct checkpoints to wait for', '1')
        .option('--timeout-min <n>', 'Max minutes to wait', '15')
        .action(async (opts) => {
            await waitCheckpoints(opts.nodeUrl, Number(opts.chainKey), Number(opts.target), Number(opts.timeoutMin));
        });

    program
        .command('verify-attestations')
        .description('Assert every active attestor appears in at least one on-chain attestation record')
        .requiredOption('--node-url <ws>', 'CC3 node WS url')
        .option('--chain-key <n>', 'Source chain key', '2')
        .action(async (opts) => {
            await verifyAttestations(opts.nodeUrl, Number(opts.chainKey));
        });

    await program.parseAsync(process.argv);
}

main()
    .then(() => process.exit(0))
    .catch((err) => {
        console.error(err);
        process.exit(1);
    });
