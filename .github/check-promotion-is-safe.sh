#!/bin/bash

# Guard for branch promotions (usc-dev -> usc-testnet -> main).
#
# Usage: check-promotion-is-safe.sh <last-ref> <target-branch>
#
# The promotion model is that the target branch only ever moves forward along
# the line of history that usc-dev builds. Anything on the target that is NOT
# in <last-ref> is about to be dropped by the rebase that follows, silently.
#
# Dropping an empty merge commit is fine and expected: GitHub's "Create a merge
# commit" button mints one on every promotion, and it carries no changes. But a
# commit with real content on the target means someone landed work directly on
# usc-testnet or main, and promoting would silently revert it. That is what this
# script is here to catch.

set -euo pipefail

LAST_REF="${1:?usage: $0 <last-ref> <target-branch>}"
TARGET_BRANCH="${2:?usage: $0 <last-ref> <target-branch>}"
TARGET_REF="refs/remotes/origin/$TARGET_BRANCH"

if ! git rev-parse -q --verify "$TARGET_REF" >/dev/null; then
    echo "INFO: origin/$TARGET_BRANCH not present locally, fetching it"
    git fetch --quiet origin "+refs/heads/$TARGET_BRANCH:$TARGET_REF"
fi

echo "INFO: promoting '$LAST_REF' onto '$TARGET_BRANCH'"

if git merge-base --is-ancestor "$TARGET_REF" "$LAST_REF"; then
    echo "PASS: origin/$TARGET_BRANCH is already an ancestor of $LAST_REF"
    echo "      This promotion is a clean fast-forward, nothing will be dropped."
    exit 0
fi

# Walk everything the target has that the promotion does not, and split it into
# commits that change nothing (safe to drop) and commits that do (not safe).
WITH_CONTENT=""
WITHOUT_CONTENT=0

while read -r sha; do
    [ -n "$sha" ] || continue

    tree=$(git rev-parse "${sha}^{tree}")
    is_empty=""

    # A merge whose tree matches any parent introduced no changes of its own.
    for parent in $(git rev-list --parents -n1 "$sha" | cut -d" " -f2-); do
        if [ "$(git rev-parse "${parent}^{tree}")" = "$tree" ]; then
            is_empty="yes"
            break
        fi
    done

    if [ -n "$is_empty" ]; then
        WITHOUT_CONTENT=$((WITHOUT_CONTENT + 1))
    else
        WITH_CONTENT="$WITH_CONTENT $sha"
    fi
done <<< "$(git rev-list "$LAST_REF..$TARGET_REF")"

echo "INFO: $WITHOUT_CONTENT empty commit(s) on $TARGET_BRANCH will be dropped (harmless)"

if [ -z "$WITH_CONTENT" ]; then
    echo "PASS: nothing with real content would be lost by this promotion"
    exit 0
fi

echo "FAIL: origin/$TARGET_BRANCH has commits with real content that are missing"
echo "      from $LAST_REF. Promoting would silently revert them:"
echo
for sha in $WITH_CONTENT; do
    git --no-pager log -1 --format="      %h %s" "$sha"
done
echo
echo "      Every change must enter at usc-dev and be promoted upward. Backport"
echo "      the commits above to usc-dev first, then re-run this workflow."
exit 1
