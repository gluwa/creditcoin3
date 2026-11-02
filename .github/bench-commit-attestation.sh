#!/usr/bin/env bash
# `commit_attestation`'s per-span slope is small next to the timing noise and often vanishes
# at the 2 steps / 2 repeats the attestation pallet is benchmarked with, which would stop
# commits past the benchmarked span from being charged more. Sweep `s` alone with more
# samples and take its slope from there. `m` is kept at 1..3 because each run costs ~0.2 s
# there against ~13 s at 99 attestors, and it can't be pinned to one value (the CLI panics).

set -e

PARTIAL=$(mktemp)
./target/release/creditcoin3-node benchmark pallet --chain dev --steps=5 --repeat=10 \
    --pallet pallet_attestation --extrinsic=commit_attestation --low=200,1 --high=500,3 \
    --wasm-execution=compiled --heap-pages=10000 --output "$PARTIAL"
python3 .github/splice-weight-slope.py pallets/attestation/src/weights.rs "$PARTIAL" commit_attestation s
rm -f "$PARTIAL"
