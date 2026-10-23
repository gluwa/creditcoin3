#!/bin/bash

# Usage: fetch-snapshot.sh <network> <out-dir>
#
# Download the latest qualified try-runtime snapshot of <network> from Azure Blob Storage
# (published by capture.sh, run by the try-runtime-snapshot CronJob in cc-networks-iac),
# check that it is the snapshot this run should use, and leave in <out-dir>:
#
#   state.snap.zst   the compressed snapshot, checksum-verified
#   manifest.json    its manifest
#
# The caller must already be logged in with the Azure CLI (az login / azure/login) as an
# identity that can read the network's snapshot container.
#
# Any reason not to use the snapshot fails with "STATE UNAVAILABLE", which is a failure
# of the validation, never a pass: missing, stale, wrong chain, wrong toolchain, or taken
# while a multi-block migration was running.
#
# Environment:
#   CONFIG          config.json path (default: .github/try-runtime/config.json)
#   MAX_AGE_HOURS   override the network's max_age_hours
#   GITHUB_OUTPUT   when set, receives spec_version, block_number, block_hash and age_minutes

set -euo pipefail

NETWORK="${1:?usage: $0 <network> <out-dir>}"
OUT_DIR="${2:?usage: $0 <network> <out-dir>}"
CONFIG="${CONFIG:-.github/try-runtime/config.json}"
GITHUB_OUTPUT="${GITHUB_OUTPUT:-/dev/null}"

unavailable() {
    echo "STATE UNAVAILABLE: $*" >&2
    exit 1
}

network() { jq -r --arg n "$NETWORK" ".networks[\$n]$1 // empty" "$CONFIG"; }

[ -n "$(network .genesis)" ] || unavailable "network '$NETWORK' is not in $CONFIG"
ACCOUNT=$(network .snapshot.storage_account)
CONTAINER=$(network .snapshot.container)
PREFIX=$(network .snapshot.prefix)
if [ -z "$ACCOUNT" ] || [ -z "$CONTAINER" ]; then
    unavailable "no snapshot storage is configured for $NETWORK in $CONFIG"
fi

EXPECTED_GENESIS=$(network .genesis)
CLI_REV=$(jq -r .cli.rev "$CONFIG")
MAX_AGE_HOURS="${MAX_AGE_HOURS:-$(network .max_age_hours)}"
GRACE_MINUTES=$(network .capture_grace_minutes)
NETWORK_PREFIX="$PREFIX/$NETWORK/${CLI_REV:0:12}"

mkdir -p "$OUT_DIR"
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

blob_download() {
    az storage blob download --account-name "$ACCOUNT" --container-name "$CONTAINER" \
        --auth-mode login --only-show-errors --no-progress --output none \
        --name "$1" --file "$2"
}

blob_download "$NETWORK_PREFIX/latest.json" "$WORK/latest.json" \
    || unavailable "no snapshot published at $ACCOUNT/$CONTAINER/$NETWORK_PREFIX for try-runtime-cli ${CLI_REV:0:12}"
MANIFEST_PATH=$(jq -r .manifest "$WORK/latest.json")
blob_download "$MANIFEST_PATH" "$OUT_DIR/manifest.json" \
    || unavailable "latest.json points at $MANIFEST_PATH, which cannot be downloaded"

manifest() { jq -r "$1" "$OUT_DIR/manifest.json"; }

[ "$(manifest .network)" = "$NETWORK" ] \
    || unavailable "manifest is for $(manifest .network), not $NETWORK"
[ "$(manifest .genesis.observed)" = "$EXPECTED_GENESIS" ] \
    || unavailable "snapshot genesis $(manifest .genesis.observed) is not $NETWORK's $EXPECTED_GENESIS"
[ "$(manifest .tooling.cli_rev)" = "$CLI_REV" ] \
    || unavailable "snapshot was taken with try-runtime-cli $(manifest .tooling.cli_rev), this run pins $CLI_REV"
[ "$(manifest .qualification.root_matches_header)" = "true" ] \
    || unavailable "the snapshot's storage root was not verified against its block header"
[ "$(manifest .migration_state.mbm_in_progress)" = "false" ] \
    || unavailable "the snapshot was taken while a multi-block migration was running"

# Age from the block's own timestamp, not from when the file was written: exporting
# yesterday's state today does not make it fresh. The grace covers the capture itself,
# which finishes a while after the block it pinned.
NOW_MS=$(($(date +%s) * 1000))
AGE_MINUTES=$(((NOW_MS - $(manifest .block.timestamp_ms)) / 60000))
LIMIT_MINUTES=$((MAX_AGE_HOURS * 60 + GRACE_MINUTES))
[ "$AGE_MINUTES" -le "$LIMIT_MINUTES" ] \
    || unavailable "the latest snapshot (block #$(manifest .block.number)) is ${AGE_MINUTES} min old; the limit is ${LIMIT_MINUTES} min"

blob_download "$(manifest .artifact.path)" "$OUT_DIR/state.snap.zst" \
    || unavailable "the snapshot file $(manifest .artifact.path) cannot be downloaded"
ZST_SHA256=$(sha256sum "$OUT_DIR/state.snap.zst" | cut -d' ' -f1)
[ "$ZST_SHA256" = "$(manifest .artifact.zst_sha256)" ] \
    || unavailable "checksum mismatch: downloaded $ZST_SHA256, manifest says $(manifest .artifact.zst_sha256)"

echo "INFO: $NETWORK snapshot of block #$(manifest .block.number) $(manifest .block.hash)," \
    "spec_version $(manifest .runtime.spec_version), ${AGE_MINUTES} min old"
{
    echo "spec_version=$(manifest .runtime.spec_version)"
    echo "block_number=$(manifest .block.number)"
    echo "block_hash=$(manifest .block.hash)"
    echo "age_minutes=$AGE_MINUTES"
} >>"$GITHUB_OUTPUT"
