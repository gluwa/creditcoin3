#!/bin/bash

set -xeuo pipefail

VERSION_FROM_CARGO_TOML=$(grep "^version =" Cargo.toml  | cut -f2 -d'=' | tr -d "' \"")
VERSION_FROM_GIT_TAG=$(git describe --tag)

# when releasing version strings in Cargo.toml and git tags must be in sync
echo "INFO: Cargo.toml version is $VERSION_FROM_CARGO_TOML"
echo "INFO: git tag version is $VERSION_FROM_GIT_TAG"

if [[ ! "$VERSION_FROM_GIT_TAG" =~ "$VERSION_FROM_CARGO_TOML"* ]]; then
    echo "FAIL: Versions in Cargo.toml and git tag are not in sync"
    exit 2
fi

echo "PASS: Versions in Cargo.toml and git tag are in sync"
exit 0
