#!/bin/bash

set -euo pipefail

MAJOR=$(grep authoring_version: runtime/src/version.rs | cut -f2 -d: | tr -d " ,")
MINOR=$(grep spec_version: runtime/src/version.rs | cut -f2 -d: | tr -d " ,")
PATCH=$(grep impl_version: runtime/src/version.rs | cut -f2 -d: | tr -d " ,")
CURRENT_VERSION="$MAJOR.$MINOR.$PATCH"

echo "INFO: current version is $CURRENT_VERSION"

NEW_MINOR=$((MINOR+1))
NEW_VERSION="$MAJOR.$NEW_MINOR.$PATCH"
echo "INFO: new version will be $NEW_VERSION"

# modify version.rs
sed -i "s/spec_version: $MINOR,/spec_version: $NEW_MINOR,/" runtime/src/version.rs

# modify Cargo.toml & Cargo.lock
sed -i "s/^version = \"$CURRENT_VERSION\"/version = \"$NEW_VERSION\"/" Cargo.toml
cargo generate-lockfile
