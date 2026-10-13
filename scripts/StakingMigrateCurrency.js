#!/usr/bin/env node

/**
 * Sweep `staking.migrateCurrency` over every stash still on the legacy staking lock.
 *
 * stable2512 (spec 132+) moved pallet-staking from a balance lock to a hold. The move is lazy:
 * each stash keeps its `staking ` lock until someone calls `migrate_currency(stash)`. Until then a
 * `withdraw_unbonded` releases a hold that does not exist and leaves the stale lock behind, so the
 * sweep should run soon after the runtime upgrade.
 *
 * The call is permissionless and fee-free ON SUCCESS ONLY: a stash that has no lock fails with
 * `AlreadyMigrated` and the signer pays the fee. Candidates are therefore selected by the lock, not
 * by the ledger, and the selection is re-read on every run, so re-running after an interruption only
 * picks up what is left.
 *
 * Usage: MIGRATE_CURRENCY_SURI=<seed> node StakingMigrateCurrency.js --url <ws-url> [options]
 *
 * Without --execute this is a dry run: it lists the candidates and the estimated force-withdrawals
 * and sends nothing.
 */

const { ApiPromise, WsProvider, Keyring } = require('@polkadot/api');
const { cryptoWaitReady } = require('@polkadot/util-crypto');
const fs = require('fs');

const SURI_ENV = 'MIGRATE_CURRENCY_SURI';
// pallet-staking's lock id, `*b"staking "`, with the trailing space.
const STAKING_LOCK_ID = '0x' + Buffer.from('staking ').toString('hex');

function parseArgs() {
    const args = process.argv.slice(2);
    const options = {
        url: null,
        execute: false,
        limit: Infinity,
        wave: 25,
        timeoutSecs: 120,
        out: null,
        ecdsa: false,
    };

    let i = 0;
    while (i < args.length) {
        if (args[i] === '--url' && i + 1 < args.length) {
            options.url = args[++i];
        } else if (args[i] === '--execute') {
            options.execute = true;
        } else if (args[i] === '--limit' && i + 1 < args.length) {
            options.limit = parseInt(args[++i], 10);
        } else if (args[i] === '--wave' && i + 1 < args.length) {
            options.wave = parseInt(args[++i], 10);
        } else if (args[i] === '--timeout' && i + 1 < args.length) {
            options.timeoutSecs = parseInt(args[++i], 10);
        } else if (args[i] === '--out' && i + 1 < args.length) {
            options.out = args[++i];
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

    if (!options.url) {
        console.error('Error: --url is required\n');
        printUsage();
        process.exit(1);
    }
    for (const [name, value] of [
        ['--limit', options.limit],
        ['--wave', options.wave],
        ['--timeout', options.timeoutSecs],
    ]) {
        if (!(value > 0)) {
            console.error(`Error: ${name} must be a positive number`);
            process.exit(1);
        }
    }
    return options;
}

function printUsage() {
    console.error(`Usage: ${SURI_ENV}=<seed> node StakingMigrateCurrency.js --url <ws-url> [options]`);
    console.error('');
    console.error('Options:');
    console.error('  --url <url>        Creditcoin3 WebSocket RPC URL (required)');
    console.error('  --execute          Send the transactions (default: dry run, sends nothing)');
    console.error('  --limit <n>        Migrate at most n stashes, e.g. a small canary first');
    console.error('  --wave <n>         Transactions in flight per wave (default: 25)');
    console.error('  --timeout <secs>   Per-wave inclusion timeout (default: 120)');
    console.error('  --out <file>       Results file (default: migrate-currency-<spec>-<time>.json)');
    console.error('  --ecdsa            Signer seed is an ECDSA key instead of sr25519');
    console.error('  -h, --help         Show this help message');
    console.error('');
    console.error(`The signer seed is read from ${SURI_ENV}, never from argv. Any funded account`);
    console.error('works; the fee is refunded for every stash that migrates successfully.');
}

/**
 * Stashes holding the legacy staking lock at `apiAt`, with what migrate_currency is expected to do.
 *
 * The runtime withdraws `max(0, ledger.total - (free - ED))` from the ledger (active first, then the
 * unlocking chunks) when the stash cannot hold its full stake. The tokens stay free, they just leave
 * the ledger. This mirrors that rule so the operator can review it before sending anything.
 */
async function findCandidates(apiAt) {
    const ed = apiAt.consts.balances.existentialDeposit.toBigInt();
    const locks = await apiAt.query.balances.locks.entries();
    const candidates = [];
    const orphans = [];

    for (const [key, accountLocks] of locks) {
        const lock = accountLocks.find((l) => l.id.toHex() === STAKING_LOCK_ID);
        if (!lock) continue;
        const stash = key.args[0].toString();

        const controller = await apiAt.query.staking.bonded(stash);
        const ledger = controller.isSome ? await apiAt.query.staking.ledger(controller.unwrap()) : null;
        if (!ledger || ledger.isNone) {
            orphans.push({ stash, lock: lock.amount.toString() });
            continue;
        }

        const total = ledger.unwrap().total.toBigInt();
        const { free, reserved } = (await apiAt.query.system.account(stash)).data;
        const freeToStake = free.toBigInt() > ed ? free.toBigInt() - ed : 0n;
        const forceWithdraw = total > freeToStake ? total - freeToStake : 0n;
        candidates.push({
            stash,
            lock: lock.amount.toString(),
            ledgerTotal: total.toString(),
            free: free.toString(),
            reserved: reserved.toString(),
            expectedForceWithdraw: forceWithdraw.toString(),
        });
    }
    return { candidates, orphans };
}

/** Whether `stash` still holds the legacy staking lock in the latest (best) state. */
async function hasStakingLock(api, stash) {
    const accountLocks = await api.query.balances.locks(stash);
    return accountLocks.some((l) => l.id.toHex() === STAKING_LOCK_ID);
}

/**
 * Wait until none of the signer's transactions are left in the pool. A call that timed out (in this
 * run or an earlier one) may still land; sending a stash again before it does would pay for an
 * `AlreadyMigrated`. Once the on-chain nonce catches up, the best-block lock check sees its effect.
 */
async function waitForIdle(api, address, timeoutMs) {
    const deadline = Date.now() + timeoutMs;
    for (let logged = false; ; logged = true) {
        const [next, account] = await Promise.all([
            api.rpc.system.accountNextIndex(address),
            api.query.system.account(address),
        ]);
        const pending = next.toNumber() - account.nonce.toNumber();
        if (pending <= 0) return true;
        if (Date.now() >= deadline) {
            console.log(`Signer still has ${pending} pending transactions after ${timeoutMs / 1000}s`);
            return false;
        }
        if (!logged) console.log(`Waiting for ${pending} pending signer transactions to land`);
        await new Promise((r) => setTimeout(r, 3000));
    }
}

function decodeError(api, dispatchError) {
    if (dispatchError.isModule) {
        const meta = api.registry.findMetaError(dispatchError.asModule);
        return `${meta.section}.${meta.name}`;
    }
    return dispatchError.toString();
}

/** Submit one migrate_currency and resolve once it is in a block, or with the reason it is not. */
function submit(api, signer, stash, nonce, timeoutMs) {
    return new Promise((resolve) => {
        let unsub = null;
        const finish = (result) => {
            clearTimeout(timer);
            if (unsub) unsub();
            resolve({ stash, nonce, ...result });
        };
        const timer = setTimeout(() => finish({ status: 'timeout' }), timeoutMs);

        api.tx.staking
            .migrateCurrency(stash)
            .signAndSend(signer, { nonce }, (r) => {
                if (r.status.isInvalid || r.status.isDropped || r.status.isUsurped) {
                    finish({ status: 'rejected', error: r.status.type });
                    return;
                }
                if (!r.status.isInBlock) return;

                const block = r.status.asInBlock.toHex();
                if (r.dispatchError) {
                    finish({ status: 'failed', block, error: decodeError(api, r.dispatchError) });
                    return;
                }
                const migrated = r.events.find(
                    ({ event: e }) => e.section === 'staking' && e.method === 'CurrencyMigrated',
                );
                finish({
                    status: 'migrated',
                    block,
                    forceWithdraw: migrated ? migrated.event.data[1].toString() : null,
                });
            })
            .then((u) => {
                unsub = u;
            })
            .catch((err) => finish({ status: 'rejected', error: err.message }));
    });
}

async function main() {
    const options = parseArgs();
    const suri = process.env[SURI_ENV];
    if (options.execute && !suri) {
        console.error(`Error: --execute needs the signer seed in ${SURI_ENV}`);
        process.exit(1);
    }

    await cryptoWaitReady();
    const api = await ApiPromise.create({ provider: new WsProvider(options.url), noInitWarn: true });
    const [chain, version] = await Promise.all([api.rpc.system.chain(), api.rpc.state.getRuntimeVersion()]);
    console.log(`Connected to ${chain}, genesis ${api.genesisHash.toHex()}, spec ${version.specVersion}`);

    if (!api.tx.staking.migrateCurrency) {
        console.error('Error: this runtime has no staking.migrateCurrency (needs the stable2512 runtime)');
        process.exit(1);
    }

    const finalized = await api.rpc.chain.getFinalizedHead();
    const header = await api.rpc.chain.getHeader(finalized);
    console.log(`Selecting candidates at finalized #${header.number} ${finalized.toHex()}`);
    const { candidates, orphans } = await findCandidates(await api.at(finalized));

    const totalForce = candidates.reduce((sum, c) => sum + BigInt(c.expectedForceWithdraw), 0n);
    const forced = candidates.filter((c) => c.expectedForceWithdraw !== '0');
    console.log(`Stashes on the legacy staking lock: ${candidates.length}`);
    console.log(`Expected force-withdrawals: ${forced.length} stashes, ${totalForce} planck in total`);
    for (const c of forced) console.log(`  ${c.stash}  ${c.expectedForceWithdraw}`);
    if (orphans.length) {
        // migrate_currency needs a ledger; these need a separate look, not this sweep.
        console.log(`Skipping ${orphans.length} staking locks with no ledger:`);
        for (const o of orphans) console.log(`  ${o.stash}  lock ${o.lock}`);
    }

    const selected = candidates.slice(0, options.limit);
    const report = {
        chain: chain.toString(),
        genesisHash: api.genesisHash.toHex(),
        specVersion: version.specVersion.toNumber(),
        selectedAt: { number: header.number.toNumber(), hash: finalized.toHex() },
        execute: options.execute,
        candidates,
        orphans,
        results: [],
    };
    const out =
        options.out || `migrate-currency-${version.specVersion}-${new Date().toISOString().replace(/[:.]/g, '-')}.json`;
    const save = () => fs.writeFileSync(out, JSON.stringify(report, null, 2));

    if (!options.execute) {
        save();
        console.log(`Dry run: nothing sent. ${selected.length} stashes would be migrated. Report: ${out}`);
        await api.disconnect();
        return 0;
    }

    // AccountId32 + MultiSignature: an ECDSA signer is `ecdsa`, not the 20-byte `ethereum` type.
    const keyring = new Keyring({ type: options.ecdsa ? 'ecdsa' : 'sr25519' });
    const signer = keyring.addFromUri(suri);
    const { free } = (await api.query.system.account(signer.address)).data;
    // The fee is taken up front and refunded on success, so a wave needs that much float.
    const fee = (
        await api.tx.staking.migrateCurrency(selected[0] ? selected[0].stash : signer.address).paymentInfo(signer)
    ).partialFee.toBigInt();
    const needed = fee * BigInt(options.wave);
    console.log(`Signer ${signer.address}, free ${free}, fee per call ${fee}, float needed per wave ${needed}`);
    if (free.toBigInt() < needed) {
        console.error('Error: signer cannot float the fees of one wave; fund it or lower --wave');
        process.exit(1);
    }

    const started = Date.now();
    let aborted = false;
    for (let i = 0; i < selected.length && !aborted; i += options.wave) {
        if (!(await waitForIdle(api, signer.address, options.timeoutSecs * 1000))) {
            aborted = true;
            break;
        }
        // Candidates come from finalized state, but a stash migrated in a block that is not final yet
        // would fail with AlreadyMigrated and cost the fee. Re-check each lock against the best block.
        const chunk = [];
        const results = [];
        for (const c of selected.slice(i, i + options.wave)) {
            if (await hasStakingLock(api, c.stash)) chunk.push(c);
            else results.push({ stash: c.stash, status: 'skipped', error: 'no longer on the legacy lock' });
        }
        // Re-read every wave: the pool's view of the next nonce, so an aborted run never leaves a gap.
        let nonce = (await api.rpc.system.accountNextIndex(signer.address)).toNumber();
        results.push(
            ...(await Promise.all(chunk.map((c) => submit(api, signer, c.stash, nonce++, options.timeoutSecs * 1000)))),
        );
        report.results.push(...results);
        save();

        const counts = {};
        for (const r of results) counts[r.status] = (counts[r.status] || 0) + 1;
        console.log(`Wave ${i / options.wave + 1}: ${JSON.stringify(counts)}`);
        for (const r of results.filter((x) => x.status !== 'migrated')) {
            console.log(`  ${r.status} ${r.stash} ${r.error || ''}`);
        }
        // A rejected or stuck tx leaves later nonces waiting in the pool; stop and let a re-run resume
        // (it waits for those to clear before sending anything).
        aborted = results.some((r) => r.status === 'rejected' || r.status === 'timeout');
    }

    let left = 0;
    for (const c of selected) {
        if (await hasStakingLock(api, c.stash)) left++;
    }
    report.summary = {
        seconds: Math.round((Date.now() - started) / 1000),
        attempted: report.results.length,
        migrated: report.results.filter((r) => r.status === 'migrated').length,
        stillOnLegacyLock: left,
        aborted,
    };
    save();
    console.log(`Done: ${JSON.stringify(report.summary)}. Report: ${out}`);
    if (aborted) console.log('Stopped early on a rejected, timed-out or pending transaction; re-run to continue.');

    await api.disconnect();
    return left === 0 && !aborted ? 0 : 1;
}

main()
    .then((code) => process.exit(code))
    .catch((err) => {
        console.error(err);
        process.exit(1);
    });
