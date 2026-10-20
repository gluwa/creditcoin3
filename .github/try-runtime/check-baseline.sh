#!/bin/bash

# Usage: check-baseline.sh <manifest.json> <network>
#
# Before a promotion, confirm the snapshot still describes the live chain: the same
# runtime code and spec_version, the same storage version for every pallet, and no
# multi-block migration running. A day-old snapshot is close enough for ordinary state
# growth, but not across a runtime upgrade or a migration, so any difference rejects it
# whatever its age. These are a handful of small reads against the network's public RPC.
#
# Environment:
#   CONFIG   config.json path (default: .github/try-runtime/config.json)

set -euo pipefail

MANIFEST="${1:?usage: $0 <manifest.json> <network>}"
NETWORK="${2:?usage: $0 <manifest.json> <network>}"
CONFIG="${CONFIG:-.github/try-runtime/config.json}"

PUBLIC_RPC=$(jq -r --arg n "$NETWORK" '.networks[$n].public_rpc // empty' "$CONFIG")
[ -n "$PUBLIC_RPC" ] || { echo "FAIL: no public RPC for '$NETWORK' in $CONFIG"; exit 1; }
# The public endpoints serve HTTP JSON-RPC on the same host as WebSocket.
RPC_URL="${PUBLIC_RPC/#ws/http}"

rpc() {
    local response
    response=$(curl -sS --fail --max-time 60 -H 'Content-Type: application/json' \
        -d "{\"id\":1,\"jsonrpc\":\"2.0\",\"method\":\"$1\",\"params\":$2}" "$RPC_URL") || {
        echo "FAIL: RPC $1 could not be reached at $RPC_URL" >&2
        return 1
    }
    jq -c '.result' <<<"$response"
}

le_to_dec() {
    local hex="${1#0x}" be="" i
    for ((i = ${#hex} - 2; i >= 0; i -= 2)); do
        be+="${hex:i:2}"
    done
    printf '%d' "0x${be:-0}"
}

HEAD=$(rpc chain_getFinalizedHead '[]' | jq -r .)
LIVE_SPEC=$(rpc state_getRuntimeVersion "[\"$HEAD\"]" | jq -r .specVersion)
LIVE_CODE_HASH=$(rpc state_getStorageHash "[\"0x3a636f6465\", \"$HEAD\"]" | jq -r .)
LIVE_CURSOR=$(rpc state_getStorage "[\"$(jq -r .storage_keys.mbm_cursor "$CONFIG")\", \"$HEAD\"]")

DIFFERENCES=()
SNAP_SPEC=$(jq -r .runtime.spec_version "$MANIFEST")
SNAP_CODE_HASH=$(jq -r .runtime.code_hash "$MANIFEST")
[ "$LIVE_SPEC" = "$SNAP_SPEC" ] \
    || DIFFERENCES+=("spec_version: snapshot $SNAP_SPEC, live $LIVE_SPEC")
[ "$LIVE_CODE_HASH" = "$SNAP_CODE_HASH" ] \
    || DIFFERENCES+=("runtime code: snapshot $SNAP_CODE_HASH, live $LIVE_CODE_HASH")
[ "$LIVE_CURSOR" = "null" ] \
    || DIFFERENCES+=("a multi-block migration is running on the live chain")

while IFS=$'\t' read -r pallet key; do
    raw=$(rpc state_getStorage "[\"$key\", \"$HEAD\"]" | jq -r .)
    if [ "$raw" = "null" ]; then live=0; else live=$(le_to_dec "$raw"); fi
    snap=$(jq -r --arg p "$pallet" '.migration_state.pallet_storage_versions[$p] // "missing"' "$MANIFEST")
    [ "$live" = "$snap" ] \
        || DIFFERENCES+=("$pallet storage version: snapshot $snap, live $live")
done < <(jq -r '.storage_keys.pallet_storage_versions | to_entries[] | "\(.key)\t\(.value)"' "$CONFIG")

if [ "${#DIFFERENCES[@]}" -gt 0 ]; then
    echo "FAIL: the snapshot no longer matches live $NETWORK at $HEAD:"
    printf '      - %s\n' "${DIFFERENCES[@]}"
    echo "      Capture a fresh snapshot (kubectl create job --from=cronjob/...) and rerun."
    exit 1
fi
echo "PASS: snapshot baseline matches live $NETWORK at $HEAD (spec_version $LIVE_SPEC)"
