#!/bin/bash

set -euo pipefail

LOG_FILE="$1"
echo "INFO: log file is '$LOG_FILE'"

if [ -z "$LOG_FILE" ]; then
    echo "ERROR: no log file specified"
    exit 1
fi

# check for errors in the logs
set +e
ERR_COUNT=$(grep -c -i "ERROR:" "$LOG_FILE")
set -e
if [ "$ERR_COUNT" -gt 0 ]; then
    echo "FAIL: found $ERR_COUNT errors in $LOG_FILE"
    exit "$ERR_COUNT"
else
    echo "PASS: no errors found in $LOG_FILE"
fi

exit 0
