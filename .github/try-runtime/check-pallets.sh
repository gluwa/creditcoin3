#!/bin/bash

# Usage: check-pallets.sh
#
# config.json lists the storage-version key of every pallet. capture.sh records those
# versions in each snapshot's manifest and check-baseline.sh compares them with the live
# chain before a promotion, so a pallet missing from the list is never compared. The list
# must name exactly the pallets in runtime/src/lib.rs's construct_runtime!.
#
# Environment:
#   CONFIG   config.json path (default: .github/try-runtime/config.json)

set -euo pipefail

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
CONFIG="${CONFIG:-$ROOT/.github/try-runtime/config.json}"
LIB="$ROOT/runtime/src/lib.rs"
# twox128(":__STORAGE_VERSION__:"), the second half of every storage-version key.
VERSION_SUFFIX=4e7b9012096b41c4eb3aaf947f6ea429

IN_RUNTIME=$(sed -n '/^construct_runtime!(/,/^);/p' "$LIB" |
    sed -nE 's/^[[:space:]]+([A-Z][A-Za-z0-9]*):.*/\1/p' | sort)
IN_CONFIG=$(jq -r '.storage_keys.pallet_storage_versions | keys[]' "$CONFIG" | sort)

if [ -z "$IN_RUNTIME" ]; then
    echo "FAIL: found no pallets in construct_runtime! in runtime/src/lib.rs"
    exit 1
fi

MISSING=$(comm -23 <(echo "$IN_RUNTIME") <(echo "$IN_CONFIG"))
EXTRA=$(comm -13 <(echo "$IN_RUNTIME") <(echo "$IN_CONFIG"))
BAD_KEYS=$(jq -r --arg s "$VERSION_SUFFIX" \
    '.storage_keys.pallet_storage_versions | to_entries[] | select(.value | test("^0x[0-9a-f]{32}" + $s + "$") | not) | .key' \
    "$CONFIG")

if [ -n "$MISSING" ] || [ -n "$EXTRA" ] || [ -n "$BAD_KEYS" ]; then
    echo "FAIL: .storage_keys.pallet_storage_versions in config.json does not match construct_runtime!"
    [ -z "$MISSING" ] || echo "      missing: $(echo "$MISSING" | paste -sd ' ' -)"
    [ -z "$EXTRA" ] || echo "      not in the runtime: $(echo "$EXTRA" | paste -sd ' ' -)"
    [ -z "$BAD_KEYS" ] || echo "      not a storage-version key: $(echo "$BAD_KEYS" | paste -sd ' ' -)"
    echo "      Each key is twox128(<pallet name>) ++ twox128(\":__STORAGE_VERSION__:\")."
    exit 1
fi
echo "PASS: config.json covers all $(echo "$IN_RUNTIME" | wc -l | tr -d ' ') pallets in construct_runtime!"
