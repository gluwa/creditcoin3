// Compare two checkpoint CSV files (each line: "block_number,digest_hash").
//
// Reports, by block number:
//   - counts in each file
//   - blocks present only in A
//   - blocks present only in B
//   - blocks present in both but with a different digest (mismatch)
//
// Exit code is 0 when the two files describe the same set of checkpoints with
// identical digests, and 1 when there is any difference (handy for CI / scripts).
//
// Usage:
//   node CompareCheckpoints.js <fileA.csv> <fileB.csv> [--limit N]
//   (--limit caps how many examples are printed per category; default 50)

const fs = require('fs');

function parseArgs() {
    const args = process.argv.slice(2);
    const result = { positional: [], limit: 50 };
    for (let i = 0; i < args.length; i++) {
        if (args[i] === '--limit' && args[i + 1]) result.limit = parseInt(args[++i], 10);
        else result.positional.push(args[i]);
    }
    return result;
}

// Parse a checkpoints CSV into a Map<blockNumber(string), digest(string)>.
// Header lines (not starting with a digit) are skipped, matching ImportCheckpoints.js.
function parseCheckpoints(file) {
    const raw = fs.readFileSync(file, 'utf8');
    const map = new Map();
    let duplicates = 0;
    for (const line of raw.trim().split('\n')) {
        const trimmed = line.trim();
        if (!trimmed) continue;
        const firstChar = trimmed[0];
        if (firstChar < '0' || firstChar > '9') continue; // skip header / comments
        const [blockNumber, digestHex] = trimmed.split(',');
        const key = blockNumber.trim();
        const digest = (digestHex || '').trim().toLowerCase();
        if (map.has(key)) duplicates++;
        map.set(key, digest);
    }
    return { map, duplicates };
}

function printCapped(label, items, limit) {
    console.log(`\n${label}: ${items.length}`);
    if (items.length === 0) return;
    const shown = items.slice(0, limit);
    for (const line of shown) console.log(`  ${line}`);
    if (items.length > shown.length) {
        console.log(`  … and ${items.length - shown.length} more (raise --limit to see all)`);
    }
}

function main() {
    const { positional, limit } = parseArgs();
    if (positional.length !== 2) {
        console.error('Usage: node CompareCheckpoints.js <fileA.csv> <fileB.csv> [--limit N]');
        process.exit(2);
    }

    const [fileA, fileB] = positional;
    const a = parseCheckpoints(fileA);
    const b = parseCheckpoints(fileB);

    console.log(`A: ${fileA} — ${a.map.size} unique blocks${a.duplicates ? ` (${a.duplicates} duplicate rows)` : ''}`);
    console.log(`B: ${fileB} — ${b.map.size} unique blocks${b.duplicates ? ` (${b.duplicates} duplicate rows)` : ''}`);

    const onlyInA = [];
    const onlyInB = [];
    const mismatched = [];

    for (const [block, digest] of a.map) {
        if (!b.map.has(block)) onlyInA.push(block);
        else if (b.map.get(block) !== digest) {
            mismatched.push(`block ${block}: A=${digest} B=${b.map.get(block)}`);
        }
    }
    for (const block of b.map.keys()) {
        if (!a.map.has(block)) onlyInB.push(block);
    }

    // Sort numerically for stable, readable output.
    const byNum = (x, y) => Number(x) - Number(y);
    onlyInA.sort(byNum);
    onlyInB.sort(byNum);

    printCapped(`Only in A (${fileA})`, onlyInA, limit);
    printCapped(`Only in B (${fileB})`, onlyInB, limit);
    printCapped('Digest mismatches (same block, different digest)', mismatched, limit);

    const identical = onlyInA.length === 0 && onlyInB.length === 0 && mismatched.length === 0;
    console.log(`\n${identical ? '✅ Files match: identical checkpoint sets and digests.' : '❌ Files differ.'}`);
    process.exit(identical ? 0 : 1);
}

main();
