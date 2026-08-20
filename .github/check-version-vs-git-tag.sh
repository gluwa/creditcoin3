#!/bin/bash

set -xeuo pipefail

VERSION_FROM_CARGO_TOML=$(grep "^version =" Cargo.toml  | cut -f2 -d'=' | tr -d "' \"")
# In CI the tag comes from the ref that triggered the workflow; `git describe`
# is only a fallback for running this by hand, since a commit can carry several
# release tags at once and describe reports just one of them.
VERSION_FROM_GIT_TAG="${TAG_NAME:-$(git describe --tag)}"

# when releasing version strings in Cargo.toml and git tags must be in sync
echo "INFO: Cargo.toml version is $VERSION_FROM_CARGO_TOML"
echo "INFO: git tag version is $VERSION_FROM_GIT_TAG"

# Anchored literal prefix plus a mandatory "-<suffix>". The previous test used
# =~ with an unanchored pattern, so Cargo.toml 3.13.0 matched tag 3.131.0-mainnet
# ('.' matching any character, trailing '*' applying to the final '0' only).
if [[ "$VERSION_FROM_GIT_TAG" != "$VERSION_FROM_CARGO_TOML"-* ]]; then
    echo "FAIL: Versions in Cargo.toml and git tag are not in sync"
    exit 2
fi

echo "PASS: Versions in Cargo.toml and git tag are in sync"
exit 0
