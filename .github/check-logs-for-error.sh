#!/bin/bash

set -euo pipefail

TARGET_FILE="$1"
echo "INFO: target file is '$TARGET_FILE'"

if [ -z "$TARGET_FILE" ]; then
    echo "ERROR: no target file specified"
    exit 1
fi

# Optional per-call allowlist. Callers that deliberately induce benign, expected error noise
# (e.g. a test that restarts a WebSocket node) pass an extended-regex via ERROR_ALLOWLIST to
# exempt *only* their own step. It defaults to empty so every other job keeps the strict gate —
# the allowlist is NOT baked into this shared script, so it can't silently weaken unrelated jobs.
ERROR_ALLOWLIST="${ERROR_ALLOWLIST:-}"
if [ -n "$ERROR_ALLOWLIST" ]; then
    echo "INFO: applying caller-supplied ERROR_ALLOWLIST: $ERROR_ALLOWLIST"

    # Reject a malformed allowlist here, loudly, instead of letting it disable the gate.
    # `grep -vE` with an invalid ERE exits >=1 having produced no output, and the filter below
    # runs under `set +e`, so the whole log would filter down to nothing and every file would
    # report "PASS" — a typo in the caller's regex would silently switch the gate off on a log
    # full of real failures. grep exits 0 (matched) or 1 (no match) for a *valid* pattern and
    # >=2 only on a usage/pattern error, so that is what we gate on. `set +e` around the probe
    # because the no-match case is exit 1, which errexit would otherwise treat as fatal.
    set +e
    printf '' | grep -qE "$ERROR_ALLOWLIST"
    allowlist_status=$?
    set -e
    if [ "$allowlist_status" -ge 2 ]; then
        echo "ERROR: ERROR_ALLOWLIST is not a valid extended regular expression: $ERROR_ALLOWLIST"
        exit 1
    fi
fi

# Filter a log to its failing ERROR lines: drop known-benign node noise, then, when the caller
# supplied one, drop their allowlisted lines too. Keeping this in one place means the count and
# the printed failures stay in sync.
filter_errors() {
    local log="$1"
    local out
    out=$(grep -i "ERROR:" "$log" \
        | grep -v "libp2p" \
        | grep -v "DEBUG tokio-runtime-worker jsonrpsee-server: WS send error: connection closed" \
        | grep -v "unable to load new segment")
    if [ -n "$ERROR_ALLOWLIST" ]; then
        out=$(printf '%s\n' "$out" | grep -vE "$ERROR_ALLOWLIST")
    fi
    printf '%s' "$out"
}

# shellcheck disable=SC2044
for LOG_FILE in $(find "$TARGET_FILE" -type f ); do
    echo "INFO: inspecting file '$LOG_FILE'"

    # check for errors in creditcoin3-node logs
    # NOTICE: ignoring libp2p connection errors
    set +e
    FILTERED=$(filter_errors "$LOG_FILE")
    ERR_COUNT=$(printf '%s' "$FILTERED" | grep -c -i "ERROR:")
    set -e
    if [[ "$ERR_COUNT" -gt 0 ]]; then
        echo "FAIL: found $ERR_COUNT errors in $LOG_FILE"
        echo "======"
        printf '%s\n' "$FILTERED"
        echo "======"
        # Exit 1, not "$ERR_COUNT": exit statuses are taken mod 256, so a log with exactly 256
        # (or 512, ...) matching lines exited 0 and the step passed while printing "FAIL". The
        # count is already reported above; the status only needs to say "failed".
        exit 1
    else
        echo "PASS: no errors found in $LOG_FILE"
    fi
done

exit 0
