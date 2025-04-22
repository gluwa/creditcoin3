#!/bin/bash

set -euo pipefail

MONITOR_FILE="$1"
echo "INFO: monitor file is '$MONITOR_FILE'"

if [ -z "$MONITOR_FILE" ]; then
    echo "ERROR: no monitor file specified"
    exit 2
fi

HANDLERS_FROM_SOURCE=$(grep handler: cc3-indexer/datasources.ts | tr -d ' ",' | tr -d "'" | cut -f2 -d: | sort | uniq)
echo "INFO: handlers defined in datasources.ts are"
echo "$HANDLERS_FROM_SOURCE"

HANDLERS_FROM_RUNTIME=$(grep "\- Handler:" "$MONITOR_FILE" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq)
echo "INFO: handlers executed during runtime are"
echo "$HANDLERS_FROM_RUNTIME"

echo "INFO: runtime execution stats"
grep "\- Handler:" "$MONITOR_FILE" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq -c

if [ "$HANDLERS_FROM_SOURCE" != "$HANDLERS_FROM_RUNTIME" ]; then
    echo "FAIL: not all handlers defined in source were executed during runtime!"
    set +e
    diff -u <(echo "$HANDLERS_FROM_SOURCE") <(echo "$HANDLERS_FROM_RUNTIME") | colordiff
    set -e
    echo "TIP: missing ones above were not executed. This is usually a sign of"
    echo "TIP: missing tests; unaccounted changes in Prover.sol and/or mistake"
    exit 3
fi

exit 0
