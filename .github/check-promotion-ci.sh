#!/usr/bin/env bash

# Promotion gate: the commit being promoted must have passed CI under promotion rules.
#
# A fast-forward has no PR, so ci.yml's promotion mode runs every test suite on the exact
# commit under the target's rules, titled "Promotion check into <target>". Extrinsics,
# stale metadata, setup and ordinary test failures block promotion. Historical compatibility
# and storage-version differences are advisory. Checks never deploy to live networks or
# commit metadata, so a dry-run promotion cannot cause those writes either.
#
# Usage: check-promotion-ci.sh <source-sha> <source-ref> <target-branch> <may-dispatch>
#
#   passed run for the commit     PASS
#   run still going               wait for it, then PASS or FAIL
#   failed run                    FAIL with its link; re-run its failed jobs, then promote again
#   no run, may-dispatch=true     dispatch one on <source-ref> (whose head must be the commit),
#                                 wait for it, then PASS or FAIL
#   no run, may-dispatch=false    FAIL (the pull_request self-test never dispatches)
#
# Needs GH_TOKEN with actions:read, and actions:write to dispatch.

set -euo pipefail

source_sha="$1"
source_ref="$2"
target="$3"
may_dispatch="$4"

title="Promotion check into ${target}"
# A full CI run takes about half an hour; leave room for a queue.
wait_limit_s=$(( 150 * 60 ))

# Retry only reads. A dispatch is not idempotent: retrying an ambiguous response could
# start another full CI run. Never turn an API failure into a passing CI verdict.
read_with_retry() {
  local attempt output delay
  for attempt in 1 2 3; do
    if output=$("$@"); then
      printf '%s\n' "$output"
      return 0
    fi
    if [ "$attempt" -lt 3 ]; then
      delay=$(( attempt * 5 ))
      echo "WARN: ${1} read failed (attempt ${attempt}/3); retrying in ${delay}s." >&2
      sleep "$delay"
    fi
  done
  echo "FAIL: ${1} read failed after 3 attempts; promotion is blocked." >&2
  return 1
}

latest_run() {
  read_with_retry gh run list --workflow ci.yml --commit "$source_sha" --event workflow_dispatch \
      --limit 50 --json databaseId,displayTitle,headSha,status,conclusion,url \
    | jq -c --arg t "$title" --arg sha "$source_sha" \
      '[.[] | select(.displayTitle == $t and .headSha == $sha)] | first // empty'
}

branch_runs() {
  read_with_retry gh run list --workflow ci.yml --branch "$source_ref" --event workflow_dispatch \
    --limit 50 --json databaseId,displayTitle,headSha,status,conclusion,url
}

wait_for() {
  local id="$1" deadline=$(( SECONDS + wait_limit_s )) status
  while :; do
    status=$(read_with_retry gh run view "$id" --json status --jq .status)
    [ "$status" = "completed" ] && return 0
    if (( SECONDS > deadline )); then
      echo "FAIL: ${title} run ${id} still ${status} after $(( wait_limit_s / 60 )) min."
      echo "      Promote again once it has finished."
      exit 1
    fi
    sleep 60
  done
}

verdict() {
  local run="$1" result conclusion url
  result=$(read_with_retry gh run view "$(jq -r .databaseId <<<"$run")" \
    --json conclusion,headSha,displayTitle,event,url)
  url=$(jq -r .url <<<"$result")
  if ! jq -e --arg sha "$source_sha" --arg t "$title" \
    '.headSha == $sha and .displayTitle == $t and .event == "workflow_dispatch"' \
    <<<"$result" >/dev/null; then
    echo "FAIL: CI run does not match source ${source_sha} and promotion target ${target}: ${url}"
    exit 1
  fi
  conclusion=$(jq -r .conclusion <<<"$result")
  if [ "$conclusion" = "success" ]; then
    echo "PASS: ${title} passed for ${source_sha}: ${url}"
    exit 0
  fi
  echo "FAIL: ${title} for ${source_sha} concluded '${conclusion}': ${url}"
  echo "      Fix or re-run its failed jobs, then promote again."
  exit 1
}

run=$(latest_run)
if [ -n "$run" ]; then
  echo "INFO: found $(jq -r .url <<<"$run") ($(jq -r .status <<<"$run"))"
  [ "$(jq -r .status <<<"$run")" = "completed" ] || wait_for "$(jq -r .databaseId <<<"$run")"
  verdict "$run"
fi

if [ "$may_dispatch" != "true" ]; then
  echo "FAIL: no '${title}' run of ci.yml for ${source_sha}."
  echo "      The promotion workflow starts one when dispatched."
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
gh workflow run ci.yml --ref "$source_ref" -f promotion_target="$target"

# The new run takes a few seconds to appear.
appearance_deadline=$(( SECONDS + 5 * 60 ))
for _ in $(seq 1 30); do
  if (( SECONDS > appearance_deadline )); then
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
verdict "$run"
