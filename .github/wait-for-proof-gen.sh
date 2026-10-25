#!/bin/bash

# Wait for proof-gen-api-server to report itself ready.
#
#   wait-for-proof-gen.sh <base-url> [log-file]
#
# Polls `/readyz`, which answers 200 once the server is connected to its source chains and
# following the CC3 chain, instead of sleeping a fixed time and hoping that was enough.
# `log-file` is optional; when the wait times out its tail is printed, see
# wait-for-ethereum.sh. Override the budget with WAIT_FOR_PROOF_GEN_TIMEOUT_SECS.

set -uo pipefail

BASE_URL=${1:-http://127.0.0.1:3100}
LOG_FILE=${2:-}

POLL_INTERVAL_SECS=2
TIMEOUT_SECS=${WAIT_FOR_PROOF_GEN_TIMEOUT_SECS:-180}
MAX_ATTEMPTS=$(( TIMEOUT_SECS / POLL_INTERVAL_SECS ))

for (( attempt = 0; attempt < MAX_ATTEMPTS; attempt++ )); do
    if [[ "$(curl -s -o /dev/null -w '%{http_code}' "${BASE_URL}/readyz")" == "200" ]]; then
        echo "INFO: ${BASE_URL} ready after $(( attempt * POLL_INTERVAL_SECS ))s"
        exit 0
    fi
    if (( attempt > 0 && (attempt * POLL_INTERVAL_SECS) % 30 == 0 )); then
        echo "INFO: ${BASE_URL} not ready after $(( attempt * POLL_INTERVAL_SECS ))s ....."
    fi
    sleep "${POLL_INTERVAL_SECS}"
done

echo "ERROR: ${BASE_URL}/readyz did not answer 200 within ${TIMEOUT_SECS}s" >&2
curl -s "${BASE_URL}/readyz" >&2 || true
echo >&2

if [[ -n "${LOG_FILE}" ]]; then
    if [[ -f "${LOG_FILE}" ]]; then
        echo "----- tail -n 100 ${LOG_FILE} -----" >&2
        tail -n 100 "${LOG_FILE}" >&2
        echo "----- end ${LOG_FILE} -----" >&2
    else
        echo "ERROR: no log at ${LOG_FILE} — the server likely never started" >&2
    fi
fi

exit 1
