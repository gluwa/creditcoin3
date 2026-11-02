#!/usr/bin/env bash
# `commit_attestation`'s per-span slope is small next to the timing noise and often vanishes
# at the 2 steps / 2 repeats the attestation pallet is benchmarked with, which would stop
# commits past the benchmarked span from being charged more. Measure it alone with more
# samples and splice it in.

set -e

PARTIAL=$(mktemp)
.github/bench.sh -p attestation -b -e commit_attestation -s 10 -r 20 -o "$PARTIAL"
python3 .github/splice-weights.py pallets/attestation/src/weights.rs "$PARTIAL" commit_attestation
rm -f "$PARTIAL"

if grep -q 'fn commit_attestation(_s' pallets/attestation/src/weights.rs; then
    echo "FAIL: commit_attestation has no per-span slope even with extra samples"
    exit 1
fi
