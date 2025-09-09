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
    set +e
    ERR_COUNT=$(grep -i "ERROR:" "$LOG_FILE" | grep -v "libp2p" | grep -v "DEBUG tokio-runtime-worker jsonrpsee-server: WS send error: connection closed" | grep -c -i "ERROR:")
    set -e
    if [[ "$ERR_COUNT" -gt 0 ]]; then
        echo "FAIL: found $ERR_COUNT errors in $LOG_FILE"
        echo "======"
        grep -i "ERROR:" "$LOG_FILE" | grep -v "libp2p" | grep -v "DEBUG tokio-runtime-worker jsonrpsee-server: WS send error: connection closed"
        echo "======"
        exit "$ERR_COUNT"
    else
        echo "PASS: no errors found in $LOG_FILE"
    fi
done

exit 0
