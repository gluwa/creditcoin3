#!/bin/bash

# Usage: run.sh <runtime.wasm> <state.snap> <checks> <blocktime-ms> <log> [on-runtime-upgrade options...]
#
# Runs try-runtime's on-runtime-upgrade against a snapshot, as try-runtime.yml checks a
# candidate runtime and try-runtime-selftest.yml checks that this check catches broken
# migrations. Both passes go to stdout and to <log>. Extra options go to both passes; the
# workflows pass none. TRY_RUNTIME overrides the CLI binary (default: try-runtime on PATH).

set -euo pipefail

WASM="${1:?runtime wasm}"
SNAPSHOT="${2:?state.snap}"
CHECKS="${3:?checks}"
BLOCKTIME="${4:?blocktime in ms}"
LOG="${5:?log file}"
shift 5
TRY_RUNTIME="${TRY_RUNTIME:-try-runtime}"

: >"$LOG"

# Two passes over the same original snapshot, because one cannot do both jobs.
#
# With multi-block migration checks on, the CLI runs the single-block migrations with no
# checks at all and applies the requested ones only after simulating blocks, by which
# point a VersionedMigration has bumped its storage version and skips its own pre/post
# hooks. So the first pass turns the multi-block simulation off to run every migration's
# pre_upgrade/post_upgrade (and try_state, unless the network's checks exclude it) on
# untouched state, plus idempotency and weights.
echo "::group::Single-block migrations with ${CHECKS} checks"
"$TRY_RUNTIME" --runtime "$WASM" \
    on-runtime-upgrade \
    --checks "$CHECKS" \
    --disable-mbm-checks \
    --blocktime "$BLOCKTIME" \
    "$@" \
    snap --path "$SNAPSHOT" \
    2>&1 | tee -a "$LOG"
echo "::endgroup::"

# The second pass simulates blocks until every multi-block migration finishes. 600 blocks
# is the CLI's own bound, spelled out so a change to it is a change here.
echo "::group::Multi-block migrations"
"$TRY_RUNTIME" --runtime "$WASM" \
    on-runtime-upgrade \
    --checks "$CHECKS" \
    --blocktime "$BLOCKTIME" \
    --mbm-max-blocks 600 \
    "$@" \
    snap --path "$SNAPSHOT" \
    2>&1 | tee -a "$LOG"
echo "::endgroup::"
