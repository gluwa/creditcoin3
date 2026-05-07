#!/bin/bash

set -euo pipefail

MONITOR_FILE="$1"
echo "INFO: monitor file is '$MONITOR_FILE'"

if [ -z "$MONITOR_FILE" ]; then
    echo "ERROR: no monitor file specified"
    exit 2
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HANDLER_WHITELIST_FILE="$SCRIPT_DIR/cc3-indexer-handler-whitelist.txt"

HANDLERS_FROM_SOURCE=$(grep handler: cc3-indexer/datasources.ts | tr -d ' ",' | tr -d "'" | cut -f2 -d: | sort | uniq)
echo "INFO: handlers defined in datasources.ts are"
echo "$HANDLERS_FROM_SOURCE"

HANDLER_WHITELIST=""
if [ -f "$HANDLER_WHITELIST_FILE" ]; then
    HANDLER_WHITELIST=$(
        grep -v '^[[:space:]]*#' "$HANDLER_WHITELIST_FILE" |
            grep -v '^[[:space:]]*$' |
            sort |
            uniq
    )
    echo "INFO: handlers allowed to be absent from runtime monitor (whitelist)"
    echo "${HANDLER_WHITELIST:-<empty>}"
fi

HANDLERS_FROM_RUNTIME=$(grep "\- Handler:" "$MONITOR_FILE" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq)
echo "INFO: handlers executed during runtime are"
echo "$HANDLERS_FROM_RUNTIME"

echo "INFO: runtime execution stats"
grep "\- Handler:" "$MONITOR_FILE" | cut -f3 -d' ' | cut -f1 -d, | sort | uniq -c

# Every datasources handler must appear in the monitor log at least once, except whitelist entries.
MISSING=$(comm -23 <(echo "$HANDLERS_FROM_SOURCE") <(echo "$HANDLERS_FROM_RUNTIME"))
UNEXPECTED=$(comm -13 <(echo "$HANDLERS_FROM_SOURCE") <(echo "$HANDLERS_FROM_RUNTIME"))

if [ -n "$HANDLER_WHITELIST" ]; then
    MISSING_DISALLOWED=$(comm -23 <(echo "$MISSING") <(echo "$HANDLER_WHITELIST"))
    MISSING_ALLOWED=$(comm -12 <(echo "$MISSING") <(echo "$HANDLER_WHITELIST"))
    if [ -n "${MISSING_ALLOWED:-}" ]; then
        echo "INFO: whitelisted handlers not observed in runtime (OK until covered by tests)"
        echo "$MISSING_ALLOWED"
    fi
else
    MISSING_DISALLOWED="$MISSING"
fi

if [ -n "${UNEXPECTED:-}" ]; then
    echo "FAIL: handlers executed at runtime but not listed in datasources.ts"
    echo "$UNEXPECTED"
    exit 3
fi

if [ -n "${MISSING_DISALLOWED:-}" ]; then
    echo "FAIL: handlers defined in datasources.ts were not executed during runtime"
    set +e
    diff -u <(echo "$HANDLERS_FROM_SOURCE") <(echo "$HANDLERS_FROM_RUNTIME") | colordiff
    set -e
    echo "TIP: missing handlers (excluding whitelist):"
    echo "$MISSING_DISALLOWED"
    echo "TIP: add integration coverage or temporarily whitelist in .github/cc3-indexer-handler-whitelist.txt"
    exit 3
fi

exit 0
