#!/bin/bash

set -euo pipefail

TARGET_FILE="$1"
echo "INFO: target file is '$TARGET_FILE'"

if [ -z "$TARGET_FILE" ]; then
    echo "ERROR: no target file specified"
    exit 1
fi

# shellcheck disable=SC2044
for LOG_FILE in $(find "$TARGET_FILE" -type f ); do
    echo "INFO: inspecting file '$LOG_FILE'"

    # check for errors in creditcoin3-node logs
    # NOTICE: ignoring libp2p connection errors
    # NOTICE: ignoring alloy WS transport / pubsub reconnect diagnostics — the attestor-network
    #         integration test deliberately restarts the eth (anvil) WebSocket node to exercise
    #         recovery, and the archiver (an alloy WS consumer) logs "connection reset by peer" /
    #         "connection refused" as it drops and reconnects. These crates only emit transport
    #         diagnostics; functional correctness is asserted separately (checkpoint compare).
    set +e
    ERR_COUNT=$(grep -i "ERROR:" "$LOG_FILE" | grep -v "libp2p" | grep -v "DEBUG tokio-runtime-worker jsonrpsee-server: WS send error: connection closed" | grep -v "unable to load new segment" | grep -vE "alloy_transport_ws|alloy_pubsub" | grep -c -i "ERROR:")
    set -e
    if [[ "$ERR_COUNT" -gt 0 ]]; then
        echo "FAIL: found $ERR_COUNT errors in $LOG_FILE"
        echo "======"
        grep -i "ERROR:" "$LOG_FILE" | grep -v "libp2p" | grep -v "DEBUG tokio-runtime-worker jsonrpsee-server: WS send error: connection closed" | grep -v "unable to load new segment" | grep -vE "alloy_transport_ws|alloy_pubsub"
        echo "======"
        exit "$ERR_COUNT"
    else
        echo "PASS: no errors found in $LOG_FILE"
    fi
done

exit 0
