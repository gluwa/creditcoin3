#!/bin/bash

set -euo pipefail

MONITOR_FILE="$1"
echo "INFO: monitor file is '$MONITOR_FILE'"

if [ -z "$MONITOR_FILE" ]; then
    echo "ERROR: no monitor file specified"
    exit 2
fi

# Handlers that require a non-zero bond to fire (DefaultMinBondRequirement=0 in dev/CI)
# are excluded from the required-execution check to avoid spurious failures.
BOND_ONLY_HANDLERS='handleEventUnbonded|handleEventWithdrawn'

HANDLERS_FROM_SOURCE=$(grep handler: cc3-indexer/datasources.ts | tr -d ' ",' | tr -d "'" | cut -f2 -d: | grep -Ev "$BOND_ONLY_HANDLERS" | sort | uniq)
echo "INFO: handlers defined in datasources.ts are (bond-only handlers excluded)"
echo "$HANDLERS_FROM_SOURCE"

HANDLERS_FROM_RUNTIME=$(grep "\- Handler:" "$MONITOR_FILE" | cut -f3 -d' ' | cut -f1 -d, | grep -Ev "$BOND_ONLY_HANDLERS" | sort | uniq)
echo "INFO: handlers executed during runtime are (bond-only handlers excluded)"
echo "$HANDLERS_FROM_RUNTIME"

echo "INFO: runtime execution stats"
grep "\- Handler:" "$MONITOR_FILE" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq -c

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
