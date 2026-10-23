#!/usr/bin/env bash

# Promotion gate: the commit being promoted must have passed try-runtime against a recent
# snapshot of the target network's state.
#
# A fast-forward has no PR, so try-runtime.yml's promotion mode checks the exact commit,
# with its own version, against the target's latest snapshot, after confirming the
# snapshot still matches the live chain. The run is titled
# "Try-runtime promotion check into <target>". Migration failures, unavailable or stale
# state, and a spec_version that would not upgrade all block promotion.
#
# Usage: check-promotion-try-runtime.sh <source-sha> <source-ref> <target-branch> <may-dispatch>
#
#   passed run for the commit, recent,  PASS
#   baseline still live
#   run still going                     wait for it, then judge it as below
#   failed run                          FAIL with its link; re-run it, then promote again
#   passed run older than MAX_AGE_HOURS treated as missing: state has grown since
#   passed run whose snapshot no        treated as missing: a runtime upgrade or migration
#   longer matches the live chain       since the run makes its evidence stale
#   no usable run, may-dispatch=true    dispatch one on <source-ref> (whose head must be
#                                       the commit), wait for it, then PASS or FAIL
#   no usable run, may-dispatch=false   FAIL (the self-test and release.yml never dispatch)
#
# A passing run is only ever accepted after comparing the snapshot it was checked against
# (its try-runtime-evidence artifact) with the live chain, including a run this script has
# just waited for.
#
# Environment:
#   GH_TOKEN        actions:read, and actions:write to dispatch
#   MAX_AGE_HOURS   how old a passing run may be (default 24, matching the snapshot limit)
#
# Patterned on check-promotion-ci.sh from the CI promotion gate.

set -euo pipefail

source_sha="$1"
source_ref="$2"
target="$3"
may_dispatch="$4"
max_age_hours="${MAX_AGE_HOURS:-24}"

scripts_dir=$(cd "$(dirname "$0")" && pwd)
network=$("$scripts_dir/network-for-ref.sh" --strict "$target")

workflow="try-runtime.yml"
title="Try-runtime promotion check into ${target}"
# A runtime build is up to an hour and the snapshot download can be large.
wait_limit_s=$((150 * 60))

# Retry only reads. A dispatch is not idempotent: retrying an ambiguous response could
# start another run. Never turn an API failure into a passing verdict.
read_with_retry() {
    local attempt output delay
    for attempt in 1 2 3; do
        if output=$("$@"); then
            printf '%s\n' "$output"
            return 0
        fi
        if [ "$attempt" -lt 3 ]; then
            delay=$((attempt * 5))
            echo "WARN: ${1} read failed (attempt ${attempt}/3); retrying in ${delay}s." >&2
            sleep "$delay"
        fi
    done
    echo "FAIL: ${1} read failed after 3 attempts; promotion is blocked." >&2
    return 1
}

# The newest run for this commit and target that is either still going or recent enough
# to speak for the chain as it is now.
latest_run() {
    local cutoff
    cutoff=$(jq -nr --argjson h "$max_age_hours" 'now - $h * 3600 | floor | todate')
    read_with_retry gh run list --workflow "$workflow" --commit "$source_sha" --event workflow_dispatch \
        --limit 50 --json databaseId,displayTitle,headSha,status,conclusion,url,createdAt |
        jq -c --arg t "$title" --arg sha "$source_sha" --arg cutoff "$cutoff" \
            '[.[] | select(.displayTitle == $t and .headSha == $sha)
                  | select(.status != "completed" or .createdAt >= $cutoff)] | first // empty'
}

branch_runs() {
    read_with_retry gh run list --workflow "$workflow" --branch "$source_ref" --event workflow_dispatch \
        --limit 50 --json databaseId,displayTitle,headSha,status,conclusion,url
}

wait_for() {
    local id="$1" deadline=$((SECONDS + wait_limit_s)) status
    while :; do
        status=$(read_with_retry gh run view "$id" --json status --jq .status)
        [ "$status" = "completed" ] && return 0
        if ((SECONDS > deadline)); then
            echo "FAIL: ${title} run ${id} still ${status} after $((wait_limit_s / 60)) min."
            echo "      Promote again once it has finished."
            exit 1
        fi
        sleep 60
    done
}

# Is the snapshot the run was checked against still a true picture of the live chain?
# Fails closed: a missing artifact or an unreachable RPC counts as "cannot confirm".
baseline_still_live() {
    local id="$1" dir
    dir=$(mktemp -d)
    if ! read_with_retry gh run download "$id" --name try-runtime-evidence --dir "$dir" >/dev/null; then
        echo "WARN: run ${id} has no try-runtime-evidence artifact (expired, or the run predates it)."
        rm -rf "$dir"
        return 1
    fi
    if ! CONFIG="$scripts_dir/try-runtime/config.json" \
        "$scripts_dir/try-runtime/check-baseline.sh" "$dir/manifest.json" "$network"; then
        rm -rf "$dir"
        return 1
    fi
    rm -rf "$dir"
}

# 0: PASS. 1: FAIL. 2: the run passed, but its evidence no longer describes the chain.
verdict() {
    local run="$1" id result conclusion url
    id=$(jq -r .databaseId <<<"$run")
    result=$(read_with_retry gh run view "$id" --json conclusion,headSha,displayTitle,event,url)
    url=$(jq -r .url <<<"$result")
    if ! jq -e --arg sha "$source_sha" --arg t "$title" \
        '.headSha == $sha and .displayTitle == $t and .event == "workflow_dispatch"' \
        <<<"$result" >/dev/null; then
        echo "FAIL: try-runtime run does not match source ${source_sha} and promotion target ${target}: ${url}"
        return 1
    fi
    conclusion=$(jq -r .conclusion <<<"$result")
    if [ "$conclusion" != "success" ]; then
        echo "FAIL: ${title} for ${source_sha} concluded '${conclusion}': ${url}"
        echo "      Its report job says whether the migration failed or the state was unavailable."
        echo "      Fix it or re-run the workflow, then promote again."
        return 1
    fi
    if ! baseline_still_live "$id"; then
        echo "INFO: ${title} passed for ${source_sha}, but its snapshot cannot be confirmed against live ${network}: ${url}"
        return 2
    fi
    echo "PASS: ${title} passed for ${source_sha} and its snapshot still matches live ${network}: ${url}"
    return 0
}

run=$(latest_run)
if [ -n "$run" ]; then
    echo "INFO: found $(jq -r .url <<<"$run") ($(jq -r .status <<<"$run"))"
    [ "$(jq -r .status <<<"$run")" = "completed" ] || wait_for "$(jq -r .databaseId <<<"$run")"
    status=0
    verdict "$run" || status=$?
    [ "$status" -eq 2 ] || exit "$status"
fi

if [ "$may_dispatch" != "true" ]; then
    echo "FAIL: no usable '${title}' run of ${workflow} for ${source_sha} in the last ${max_age_hours} h."
    echo "      promote-branch.yml starts one when dispatched. For a commit that was not"
    echo "      promoted (a hotfix), dispatch ${workflow} on the branch at that commit with"
    echo "      promotion_target=${target}."
    exit 1
fi

# Remember existing runs so a newly dispatched wrong-SHA run is detected explicitly,
# rather than waiting five minutes for an exact-SHA run that will never appear.
before_ids=$(branch_runs | jq -c '[.[].databaseId]')

# workflow_dispatch resolves the live branch, not checkout's remote-tracking ref. Read
# origin immediately before dispatch, then check the new run's SHA to cover the race left
# between this read and GitHub accepting the dispatch.
if ! head_sha=$(read_with_retry git ls-remote --exit-code origin "refs/heads/${source_ref}" | cut -f1); then
    echo "FAIL: cannot read live branch ${source_ref}. Dispatch requires a branch at ${source_sha}."
    exit 1
fi
if [ "$head_sha" != "$source_sha" ]; then
    echo "FAIL: source branch ${source_ref} moved or is not at the requested commit."
    echo "      Expected ${source_sha}, live head is ${head_sha}. Nothing was dispatched."
    echo "      Resolve the source again and restart promotion."
    exit 1
fi

echo "INFO: starting '${title}' on ${source_ref} (${source_sha})"
gh workflow run "$workflow" --ref "$source_ref" -f promotion_target="$target"

# The new run takes a few seconds to appear.
appearance_deadline=$((SECONDS + 5 * 60))
for _ in $(seq 1 30); do
    if ((SECONDS > appearance_deadline)); then
        break
    fi
    run=$(branch_runs | jq -c --arg t "$title" --argjson before "$before_ids" \
        '[.[] | select(.displayTitle == $t) | select(.databaseId as $id | $before | index($id) | not)] | first // empty')
    [ -n "$run" ] && break
    sleep 10
done
if [ -z "$run" ]; then
    echo "FAIL: dispatched '${title}' but the run did not appear within 5 min."
    exit 1
fi
if [ "$(jq -r .headSha <<<"$run")" != "$source_sha" ]; then
    echo "FAIL: source branch ${source_ref} moved during dispatch."
    echo "      Expected ${source_sha}; dispatched $(jq -r .headSha <<<"$run"): $(jq -r .url <<<"$run")"
    echo "      This run cannot authorize promotion. Resolve the source again and restart."
    exit 1
fi
echo "INFO: waiting for $(jq -r .url <<<"$run")"
wait_for "$(jq -r .databaseId <<<"$run")"
status=0
verdict "$run" || status=$?
if [ "$status" -eq 2 ]; then
    echo "FAIL: the check passed, but its snapshot cannot be confirmed against live ${network}"
    echo "      (the chain changed while it ran, or the RPC could not be read). Promote again."
    exit 1
fi
exit "$status"
