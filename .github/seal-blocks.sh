#!/bin/bash

# Seal and finalize a block every INTERVAL seconds on a node started with `--sealing manual`,
# so the chain moves at its normal pace. Tests that need to be further ahead (`waitEras`)
# seal extra blocks themselves instead of waiting for this loop.
#
# With FINALIZE_URL, also finalize each sealed block on that node, a second manual-sealing
# node that only imports blocks (the CLI tests' Bob).
TARGET_URL=${1:-http://127.0.0.1:9944}
INTERVAL=${2:-5}
FINALIZE_URL=${3:-}

rpc() {
    curl --silent --show-error --max-time 30 \
        -H 'Content-Type: application/json' \
        -d "{\"id\":1, \"jsonrpc\":\"2.0\", \"method\":\"$2\", \"params\":$3}" \
        "$1"
}

while true; do
    if ! reply=$(rpc "$TARGET_URL" engine_createBlock '[true, true, null]'); then
        echo "WARNING: engine_createBlock failed"
    elif [ -n "$FINALIZE_URL" ]; then
        # Bob may not have imported it yet; finalizing a later block finalizes this one too
        hash=$(jq -r '.result.hash // empty' <<<"$reply")
        [ -n "$hash" ] && rpc "$FINALIZE_URL" engine_finalizeBlock "[\"$hash\", null]" >/dev/null
    fi
    sleep "$INTERVAL"
done
