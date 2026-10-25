#!/bin/bash

set -euo pipefail

# One or more SubQuery monitor files; a sharded run passes one per shard and the handlers
# executed are their union.
if [ "$#" -eq 0 ]; then
    echo "ERROR: no monitor file specified"
    exit 2
fi
echo "INFO: monitor files are: $*"

# Exclude handlers not exercised by current integration tests (see CSUB-2041 for ForwardCheckpointPatchApplied coverage).
HANDLERS_FROM_SOURCE=$(
    grep handler: cc3-indexer/datasources.ts |
        tr -d ' ",' |
        tr -d "'" |
        cut -f2 -d: |
        grep -vx handleEventForwardCheckpointPatchApplied |
        sort |
        uniq
)
echo "INFO: handlers defined in datasources.ts are"
echo "$HANDLERS_FROM_SOURCE"

HANDLERS_FROM_RUNTIME=$(grep -h "\- Handler:" "$@" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq)
echo "INFO: handlers executed during runtime are"
echo "$HANDLERS_FROM_RUNTIME"

echo "INFO: runtime execution stats"
grep -h "\- Handler:" "$@" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq -c

if [ "$HANDLERS_FROM_SOURCE" != "$HANDLERS_FROM_RUNTIME" ]; then
    echo "FAIL: not all handlers defined in source were executed during runtime!"
    set +e
    diff -u <(echo "$HANDLERS_FROM_SOURCE") <(echo "$HANDLERS_FROM_RUNTIME") | colordiff
    set -e
    echo "TIP: missing ones above were not executed. This is usually a sign of"
    echo "TIP: missing tests or a mistake in the test suite"
    exit 3
fi

exit 0
