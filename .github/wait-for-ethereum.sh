#!/bin/bash

# Wait for an Ethereum JSON-RPC endpoint to start answering.
#
#   wait-for-ethereum.sh <url> [log-file]
#
# `log-file` is optional. When the wait times out its tail is printed, because the node's
# stdout normally goes to a file under /var/tmp that is only reachable through the uploaded
# artifact — without it a node that fails to start is indistinguishable from a slow one.
#
# The budget is deliberately modest. A node that is coming up answers inside a few seconds;
# one that failed to bind its port never will, and a long budget only turns a fast failure
# into an expensive silent one. Override with WAIT_FOR_ETHEREUM_TIMEOUT_SECS if a caller
# genuinely needs longer.

set -uo pipefail

TARGET_URL=${1:-http://127.0.0.1:8545}
LOG_FILE=${2:-}

POLL_INTERVAL_SECS=2
TIMEOUT_SECS=${WAIT_FOR_ETHEREUM_TIMEOUT_SECS:-300}
MAX_ATTEMPTS=$(( TIMEOUT_SECS / POLL_INTERVAL_SECS ))

probe() {
    # Note: both WS and HTTP are served via the same port.
    curl -s -o /dev/null -w '%{http_code}' \
        -H 'Content-Type: application/json' \
        -d '{"jsonrpc":"2.0","method":"web3_clientVersion","params":[],"id":67}' \
        "${TARGET_URL}"
}

for (( attempt = 0; attempt < MAX_ATTEMPTS; attempt++ )); do
    if [[ "$(probe)" == "200" ]]; then
        echo "INFO: ${TARGET_URL} ready after $(( attempt * POLL_INTERVAL_SECS ))s"
        exit 0
    fi
    # Report every 30s rather than every attempt: at a 2s interval a per-attempt line would
    # bury the rest of the step in noise for a wait that is usually over almost immediately.
    if (( attempt > 0 && (attempt * POLL_INTERVAL_SECS) % 30 == 0 )); then
        echo "INFO: ${TARGET_URL} not ready after $(( attempt * POLL_INTERVAL_SECS ))s ....."
    fi
    sleep "${POLL_INTERVAL_SECS}"
done

echo "ERROR: ${TARGET_URL} did not answer web3_clientVersion within ${TIMEOUT_SECS}s" >&2

if [[ -n "${LOG_FILE}" ]]; then
    if [[ -f "${LOG_FILE}" ]]; then
        echo "----- tail -n 100 ${LOG_FILE} -----" >&2
        tail -n 100 "${LOG_FILE}" >&2
        echo "----- end ${LOG_FILE} -----" >&2
    else
        echo "ERROR: no log at ${LOG_FILE} — the node likely never started" >&2
    fi
fi

exit 1
