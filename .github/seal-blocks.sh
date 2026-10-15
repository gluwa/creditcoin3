#!/bin/bash

# Seal and finalize a block every INTERVAL seconds on a node started with `--sealing manual`,
# so the chain moves at its normal pace. Tests that need to be further ahead (`waitEras`)
# seal extra blocks themselves instead of waiting for this loop.
TARGET_URL=${1:-http://127.0.0.1:9944}
INTERVAL=${2:-5}

while true; do
    curl --silent --show-error --max-time 30 -o /dev/null \
        -H 'Content-Type: application/json' \
        -d '{"id":1, "jsonrpc":"2.0", "method":"engine_createBlock", "params":[true, true, null]}' \
        "$TARGET_URL" || echo "WARNING: engine_createBlock failed"
    sleep "$INTERVAL"
done
