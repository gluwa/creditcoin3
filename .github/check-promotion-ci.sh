#!/usr/bin/env bash

# Promotion gate: the commit being promoted must have passed CI under promotion rules.
#
# A promotion is a fast-forward push, so no pull request into usc-testnet or main exists and
# the checks that block only promotion PRs (extrinsics, pallet storage versions,
# attestor-compatibility) would never get to block. ci.yml has a promotion mode for this:
# dispatched with promotion_target, it runs every suite on the dispatched commit with the
# target's rules, under the title "Promotion check into <target>".
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

latest_run() {
  gh run list --workflow ci.yml --commit "$source_sha" --event workflow_dispatch \
      --limit 50 --json databaseId,displayTitle,status,conclusion,url \
    | jq -c --arg t "$title" '[.[] | select(.displayTitle == $t)] | first // empty'
}

wait_for() {
  local id="$1" deadline=$(( SECONDS + wait_limit_s )) status
  while :; do
    status=$(gh run view "$id" --json status --jq .status)
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
  local run="$1" conclusion url
  conclusion=$(gh run view "$(jq -r .databaseId <<<"$run")" --json conclusion --jq .conclusion)
  url=$(jq -r .url <<<"$run")
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

# workflow_dispatch runs a branch's head, so that head must be the commit being promoted.
head_sha=$(git rev-parse "refs/remotes/origin/${source_ref}" 2>/dev/null || true)
if [ "$head_sha" != "$source_sha" ]; then
  echo "FAIL: no '${title}' run for ${source_sha}, and ${source_ref} is not a branch whose head"
  echo "      is that commit, so one cannot be started here. Promote a branch head, or run"
  echo "      ci.yml on a branch at ${source_sha} with promotion_target=${target} first."
  exit 1
fi

echo "INFO: starting '${title}' on ${source_ref} (${source_sha})"
gh workflow run ci.yml --ref "$source_ref" -f promotion_target="$target"

# The new run takes a few seconds to appear.
for _ in $(seq 1 30); do
  run=$(latest_run)
  [ -n "$run" ] && break
  sleep 10
done
if [ -z "$run" ]; then
  echo "FAIL: dispatched '${title}' but the run did not appear within 5 min."
  exit 1
fi
echo "INFO: waiting for $(jq -r .url <<<"$run")"
wait_for "$(jq -r .databaseId <<<"$run")"
verdict "$run"
