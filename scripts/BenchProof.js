#!/usr/bin/env node

/**
 * Benchmark proof generation: same tx hash against two prover binaries, compare
 * timing and verify both return the same proof.
 *
 * Hits the exact same endpoint SubmitProof.js uses:
 *   GET <apiUrl>/api/v1/proof-by-tx/<chainKey>/<txHash>
 *
 * Usage:
 *   node BenchProof.js <chainKey> <txHash> [options]
 *   node BenchProof.js 3 0x... --candidate-url http://localhost:3100 --baseline-url http://localhost:3101
 *
 * Options:
 *   --candidate-url <url> Current/new prover URL (default: http://localhost:3100)
 *   --baseline-url <url>  Baseline/old prover URL (default: http://localhost:3101)
 *   --local-url <url>     Alias for --candidate-url
 *   --testnet-url <url>   Alias for --baseline-url
 *   --runs <n>            Calls per prover (default: 1). First call is cold,
 *                         later calls hit each prover's cache (cached=true).
 *   --timeout <ms>        Per-request timeout (default: 120000)
 *   --no-verify          Skip proof-equality check between the two provers
 *   -v, --verbose        Print proof field sizes
 *
 * NOTE: each prover caches independently. For a fair generation-speed test use a
 * tx hash NEITHER prover has proven yet — the script flags cached=true so you can
 * tell whether a run measured generation or a cache hit.
 */

const DEFAULTS = {
    candidateUrl: 'http://localhost:3100',
    baselineUrl: 'http://localhost:3101',
    runs: 1,
    timeout: 120000,
};

function parseArgs() {
    const args = process.argv.slice(2);
    const o = {
        chainKey: null,
        txHash: null,
        candidateUrl: DEFAULTS.candidateUrl,
        baselineUrl: DEFAULTS.baselineUrl,
        runs: DEFAULTS.runs,
        timeout: DEFAULTS.timeout,
        verify: true,
        verbose: false,
    };
    let i = 0;
    while (i < args.length) {
        const a = args[i];
        if ((a === '--candidate-url' || a === '--local-url') && i + 1 < args.length) o.candidateUrl = args[++i];
        else if ((a === '--baseline-url' || a === '--testnet-url') && i + 1 < args.length) o.baselineUrl = args[++i];
        else if (a === '--runs' && i + 1 < args.length) o.runs = parseInt(args[++i], 10);
        else if (a === '--timeout' && i + 1 < args.length) o.timeout = parseInt(args[++i], 10);
        else if (a === '--no-verify') o.verify = false;
        else if (a === '-v' || a === '--verbose') o.verbose = true;
        else if (!o.chainKey) o.chainKey = a;
        else if (!o.txHash) o.txHash = a;
        i++;
    }
    if (!o.chainKey || !o.txHash) {
        console.error(
            'Usage: node BenchProof.js <chainKey> <txHash> [--candidate-url <url>] [--baseline-url <url>] [--runs <n>] [--no-verify] [-v]',
        );
        process.exit(1);
    }
    // Strip trailing slashes so "<url>/" doesn't produce a double-slash 404.
    o.candidateUrl = o.candidateUrl.replace(/\/+$/, '');
    o.baselineUrl = o.baselineUrl.replace(/\/+$/, '');
    return o;
}

// Single timed GET. Returns { ok, ms, status, body, cached }.
async function timedFetch(apiUrl, chainKey, txHash, timeout) {
    const url = `${apiUrl}/api/v1/proof-by-tx/${chainKey}/${txHash}`;
    const ctrl = new AbortController();
    const t = setTimeout(() => ctrl.abort(), timeout);
    const start = process.hrtime.bigint();
    try {
        const res = await fetch(url, {
            method: 'GET',
            headers: { 'Content-Type': 'application/json' },
            signal: ctrl.signal,
        });
        const text = await res.text();
        const ms = Number(process.hrtime.bigint() - start) / 1e6;
        let body = null;
        try {
            body = JSON.parse(text);
        } catch {
            body = text;
        }
        return { ok: res.ok, ms, status: res.status, body, cached: body && body.cached };
    } catch (err) {
        const ms = Number(process.hrtime.bigint() - start) / 1e6;
        return { ok: false, ms, status: 0, body: err.message, cached: undefined };
    } finally {
        clearTimeout(t);
    }
}

// Run a prover N times, return per-run results.
async function benchProver(label, apiUrl, opts) {
    console.log(`\n[${label}] ${apiUrl}`);
    const runs = [];
    for (let i = 0; i < opts.runs; i++) {
        const r = await timedFetch(apiUrl, opts.chainKey, opts.txHash, opts.timeout);
        runs.push(r);
        if (r.ok) {
            console.log(
                `  run ${i + 1}/${opts.runs}: ${r.ms.toFixed(1)} ms  cached=${r.cached}  headerNumber=${r.body && r.body.headerNumber}`,
            );
        } else {
            console.log(
                `  run ${i + 1}/${opts.runs}: ${r.ms.toFixed(1)} ms  ✗ status=${r.status}  ${typeof r.body === 'string' ? r.body : JSON.stringify(r.body)}`,
            );
        }
    }
    const okRuns = runs.filter((r) => r.ok);
    const times = okRuns.map((r) => r.ms);
    const stats = times.length
        ? {
              cold: runs[0].ok ? runs[0].ms : null,
              min: Math.min(...times),
              max: Math.max(...times),
              avg: times.reduce((a, b) => a + b, 0) / times.length,
          }
        : null;
    return { label, runs, okRuns, stats, proof: okRuns.length ? okRuns[0].body : null };
}

// Normalize a proof for equality comparison: drop volatile fields.
function normalizeProof(p) {
    if (!p || typeof p !== 'object') return p;
    const { cached: _cached, generatedAt: _generatedAt, ...rest } = p;
    return JSON.stringify(rest, Object.keys(rest).sort());
}

async function main() {
    const opts = parseArgs();
    console.log('=== Proof Benchmark ===');
    console.log(`Chain Key: ${opts.chainKey}`);
    console.log(`Tx:        ${opts.txHash}`);
    console.log(`Runs:      ${opts.runs} per prover`);

    const candidate = await benchProver('CANDIDATE', opts.candidateUrl, opts);
    const baseline = await benchProver('BASELINE', opts.baselineUrl, opts);

    // Proof equality
    console.log('\n=== Proof equality ===');
    if (opts.verify && candidate.proof && baseline.proof) {
        const same = normalizeProof(candidate.proof) === normalizeProof(baseline.proof);
        if (same) {
            console.log('✅ identical proof from both provers (ignoring cached/generatedAt)');
        } else {
            console.log('❌ proofs DIFFER between provers');
            if (opts.verbose) {
                const fields = ['headerNumber', 'txBytes', 'continuityProof', 'merkleProof'];
                for (const f of fields) {
                    const a = JSON.stringify(candidate.proof[f]);
                    const b = JSON.stringify(baseline.proof[f]);
                    console.log(
                        `  ${f}: ${a === b ? 'match' : 'DIFFER'} (candidate ${a ? a.length : 0}B vs baseline ${b ? b.length : 0}B)`,
                    );
                }
            }
        }
    } else if (!opts.verify) {
        console.log('(skipped --no-verify)');
    } else {
        console.log('⚠️  cannot compare — one prover returned no proof');
    }

    // Timing summary
    console.log('\n=== Timing ===');
    for (const p of [candidate, baseline]) {
        if (p.stats) {
            const coldNote = p.runs[0].ok
                ? `cold=${p.stats.cold.toFixed(1)}ms (cached=${p.runs[0].cached})`
                : 'cold=FAILED';
            console.log(
                `${p.label.padEnd(8)} ${coldNote}  | warm min/avg/max = ${p.stats.min.toFixed(1)}/${p.stats.avg.toFixed(1)}/${p.stats.max.toFixed(1)} ms  (${p.okRuns.length}/${opts.runs} ok)`,
            );
        } else {
            console.log(`${p.label.padEnd(8)} all runs failed`);
        }
    }

    if (candidate.stats && baseline.stats) {
        const cc = candidate.stats.cold ?? candidate.stats.min;
        const bc = baseline.stats.cold ?? baseline.stats.min;
        const faster = cc < bc ? 'CANDIDATE' : 'BASELINE';
        const ratio = (Math.max(cc, bc) / Math.min(cc, bc)).toFixed(2);
        console.log(
            `\n→ Cold: CANDIDATE ${cc.toFixed(1)}ms vs BASELINE ${bc.toFixed(1)}ms  → ${faster} ${ratio}x faster`,
        );
        if (candidate.runs[0].cached || baseline.runs[0].cached) {
            console.log(
                '  ⚠️  a "cold" run reported cached=true — that prover already had this proof; use a fresh tx for a true generation benchmark.',
            );
        }
    }
}

main().catch((e) => {
    console.error('Unhandled error:', e);
    process.exit(1);
});
