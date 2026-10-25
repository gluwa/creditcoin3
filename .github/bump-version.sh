#!/bin/bash

# Usage: bump-version.sh [highest-released-version]
#
# Bumps spec_version by one. When the highest version already released to the network
# is given, the bump starts from there instead: a release cut from a hotfix branch takes
# a spec_version that dev has not reached yet, and dev's next release must step over it.

set -euo pipefail

RELEASED_VERSION="${1:-}"

GITHUB_OUTPUT=${GITHUB_OUTPUT:-/dev/null}
MAJOR=$(grep authoring_version: runtime/src/version.rs | cut -f2 -d: | tr -d " ,")
MINOR=$(grep spec_version: runtime/src/version.rs | cut -f2 -d: | tr -d " ,")
PATCH=$(grep impl_version: runtime/src/version.rs | cut -f2 -d: | tr -d " ,")
CURRENT_VERSION="$MAJOR.$MINOR.$PATCH"

echo "INFO: current version is $CURRENT_VERSION"

BASE_MINOR=$MINOR
if [ -n "$RELEASED_VERSION" ]; then
    RELEASED_MINOR=$(echo "$RELEASED_VERSION" | cut -f2 -d.)
    echo "INFO: highest released version is $RELEASED_VERSION"
    if [ "$RELEASED_MINOR" -gt "$BASE_MINOR" ]; then
        BASE_MINOR=$RELEASED_MINOR
    fi
fi

NEW_MINOR=$((BASE_MINOR+1))
NEW_VERSION="$MAJOR.$NEW_MINOR.0"
echo "INFO: new version will be $NEW_VERSION"
echo "new_version=$NEW_VERSION" >> "$GITHUB_OUTPUT"

# modify version.rs
sed -i -e "s/spec_version: $MINOR,/spec_version: $NEW_MINOR,/" \
       -e "s/impl_version: $PATCH,/impl_version: 0,/" runtime/src/version.rs

# modify Cargo.toml & Cargo.lock
sed -i "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" Cargo.toml
cargo update --workspace
