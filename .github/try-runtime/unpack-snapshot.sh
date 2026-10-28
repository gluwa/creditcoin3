#!/bin/bash

# Usage: unpack-snapshot.sh <dir>
#
# Unpacks <dir>/state.snap.zst, as fetch-snapshot.sh left it, into <dir>/state.snap, and
# checks both files against <dir>/manifest.json, so a snapshot changed between jobs or
# damaged in unpacking is never tested.

set -euo pipefail

DIR="${1:?snapshot directory}"
MANIFEST="$DIR/manifest.json"

sha256() { sha256sum "$1" | cut -d' ' -f1; }

command -v zstd >/dev/null || sudo apt-get install -y zstd
if [ "$(sha256 "$DIR/state.snap.zst")" != "$(jq -r .artifact.zst_sha256 "$MANIFEST")" ]; then
    echo "FAIL: the snapshot changed between jobs"
    exit 1
fi
zstd -d --rm "$DIR/state.snap.zst" -o "$DIR/state.snap"
if [ "$(sha256 "$DIR/state.snap")" != "$(jq -r .artifact.raw_sha256 "$MANIFEST")" ]; then
    echo "FAIL: the unpacked snapshot does not match its manifest"
    exit 1
fi
jq '{network, block, runtime: {spec_version: .runtime.spec_version, code_hash: .runtime.code_hash}}' "$MANIFEST"
