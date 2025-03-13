#!/bin/bash

set -euo pipefail

LOG_FILE=$(mktemp /var/tmp/subql-node-test-XXXX.log)
echo "INFO: log file is $LOG_FILE"

TZ=UTC yarn test | tee -a "$LOG_FILE"
RESULT=$?

if [ $RESULT -gt 0 ]; then
    echo "FAIL: Tests failed"
    exit $RESULT
fi

# check for errors in the logs
set +e
ERR_COUNT=$(grep "ERROR" "$LOG_FILE"  | grep -c "failed to fetch Block hash")
set -e
if [ "$ERR_COUNT" -gt 0 ]; then
    echo "FAIL: found $ERR_COUNT errors in test log"
    exit "$ERR_COUNT"
fi

exit 0
