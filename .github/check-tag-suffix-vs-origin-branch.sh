#!/bin/bash

# Verify that a release tag was cut from the branch its suffix names:
#
#     *-devnet   ->  usc-dev
#     *-testnet  ->  usc-testnet
#     *-mainnet  ->  main
#
# We ask "is this commit contained in the branch the suffix names?" rather than
# "which branch is this commit on?". Once branches are promoted by fast-forward
# a release commit is reachable from many branches at once, so asking which
# branch it is "on" has no single answer.

set -euo pipefail

# In CI the tag comes from the ref that triggered the workflow. Fall back to
# `git describe` so the script stays runnable by hand on a tagged checkout.
GIT_TAG="${TAG_NAME:-$(git describe --tag)}"
SUFFIX_FROM_GIT_TAG=$(echo "$GIT_TAG" | cut -d"-" -f2,99)

case "$SUFFIX_FROM_GIT_TAG" in
    devnet)  EXPECTED_BRANCH="usc-dev" ;;
    testnet) EXPECTED_BRANCH="usc-testnet" ;;
    mainnet) EXPECTED_BRANCH="main" ;;
    *)
        echo "FAIL: '$GIT_TAG' has no recognized network suffix"
        echo "      expected one of: devnet, testnet, mainnet; got '$SUFFIX_FROM_GIT_TAG'"
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

echo "FAIL: $GIT_TAG is not contained in origin/$EXPECTED_BRANCH"
echo "      A '$SUFFIX_FROM_GIT_TAG' release must be tagged on a commit that is"
echo "      already merged into $EXPECTED_BRANCH. Promote the branch first, then tag."
exit 1
