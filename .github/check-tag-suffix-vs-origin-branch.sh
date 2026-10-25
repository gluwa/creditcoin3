#!/bin/bash

# Verify that a release tag was cut from the branch its suffix names:
#
#     *-devnet      ->  dev (usc-dev until the rename)
#     *-asc-devnet  ->  asc-dev (writeability-off-usc-dev until the rename)
#     *-testnet     ->  usc-testnet
#     *-mainnet     ->  main
#
# We ask "is this commit contained in the branch the suffix names?" rather than
# "which branch is this commit on?". Once branches are promoted by fast-forward
# a release commit is reachable from many branches at once, so asking which
# branch it is "on" has no single answer.
#
# Devnet hotfixes are the exception: a devnet release may be cut from any branch,
# as long as it builds on the highest devnet release so far. Devnet then never
# loses code it already runs, while dev commits that are not ready stay out.

set -euo pipefail

# In CI the tag comes from the ref that triggered the workflow. Fall back to
# `git describe` so the script stays runnable by hand on a tagged checkout.
GIT_TAG="${TAG_NAME:-$(git describe --tag)}"
SUFFIX_FROM_GIT_TAG=$(echo "$GIT_TAG" | cut -d"-" -f2-)

case "$SUFFIX_FROM_GIT_TAG" in
    devnet)     EXPECTED_BRANCH=$("$(dirname "$0")/branch-on-origin.sh" dev usc-dev) ;;
    asc-devnet) EXPECTED_BRANCH=$("$(dirname "$0")/branch-on-origin.sh" asc-dev writeability-off-usc-dev) ;;
    testnet)    EXPECTED_BRANCH="usc-testnet" ;;
    mainnet)    EXPECTED_BRANCH="main" ;;
    *)
        echo "FAIL: '$GIT_TAG' has no recognized network suffix"
        echo "      expected one of: devnet, asc-devnet, testnet, mainnet; got '$SUFFIX_FROM_GIT_TAG'"
        exit 1
        ;;
esac

# Resolve the tag to a commit; on a detached checkout of the tag HEAD will do.
if git rev-parse -q --verify "${GIT_TAG}^{commit}" >/dev/null; then
    TAGGED_COMMIT=$(git rev-parse "${GIT_TAG}^{commit}")
else
    TAGGED_COMMIT=$(git rev-parse "HEAD^{commit}")
    echo "INFO: tag '$GIT_TAG' not present locally, falling back to HEAD"
fi

# Shallow clones and tag-only fetches may not carry the branch we need.
if ! git rev-parse -q --verify "refs/remotes/origin/$EXPECTED_BRANCH" >/dev/null; then
    echo "INFO: origin/$EXPECTED_BRANCH not present locally, fetching it"
    git fetch --quiet origin "+refs/heads/$EXPECTED_BRANCH:refs/remotes/origin/$EXPECTED_BRANCH"
fi

echo "INFO: git tag: '$GIT_TAG'"
echo "INFO: suffix from git tag: '$SUFFIX_FROM_GIT_TAG'"
echo "INFO: expected branch: 'origin/$EXPECTED_BRANCH'"
echo "INFO: tagged commit: '$TAGGED_COMMIT'"

if git merge-base --is-ancestor "$TAGGED_COMMIT" "refs/remotes/origin/$EXPECTED_BRANCH"; then
    echo "PASS: $GIT_TAG is contained in origin/$EXPECTED_BRANCH"
    exit 0
fi

if [ "$SUFFIX_FROM_GIT_TAG" = "devnet" ]; then
    git fetch --quiet --tags origin 2>/dev/null || true
    LATEST_DEVNET_TAG=$(git tag -l '*-devnet' | grep -E '^[0-9]+\.[0-9]+\.[0-9]+-devnet$' | grep -vxF "$GIT_TAG" | sort -V | tail -n1 || true)
    echo "INFO: '$GIT_TAG' is not on $EXPECTED_BRANCH, checking it builds on the latest devnet release '$LATEST_DEVNET_TAG'"

    if [ -n "$LATEST_DEVNET_TAG" ] && git merge-base --is-ancestor "refs/tags/$LATEST_DEVNET_TAG" "$TAGGED_COMMIT"; then
        echo "PASS: $GIT_TAG is a hotfix on top of $LATEST_DEVNET_TAG"
        exit 0
    fi

    echo "FAIL: devnet hotfix $GIT_TAG does not build on $LATEST_DEVNET_TAG"
    echo "      Releasing it would drop changes devnet already runs."
    echo "      Branch the hotfix from $LATEST_DEVNET_TAG, or release from $EXPECTED_BRANCH."
    exit 1
fi

echo "FAIL: $GIT_TAG is not contained in origin/$EXPECTED_BRANCH"
echo "      A '$SUFFIX_FROM_GIT_TAG' release must be tagged on a commit that is"
echo "      already merged into $EXPECTED_BRANCH. Promote the branch first, then tag."
exit 1
