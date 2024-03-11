#!/bin/bash

set -euo pipefail

GIT_TAG=$(git describe --tag)
SUFFIX_FROM_GIT_TAG=$(echo "$GIT_TAG" | cut -d"-" -f2,99)

echo "----- DEBUG -----"
git branch -a --contains
echo "----- DEBUG -----"
git branch -a --contains | grep remotes/origin
echo "----- END -----"

NEAREST_GIT_BRANCH=$(git branch -a --contains | grep remotes/origin | cut -f3 -d/)

echo "INFO: git tag: '$GIT_TAG'"
echo "INFO: suffix from git tag: '$SUFFIX_FROM_GIT_TAG'"
echo "INFO: nearest git branch '$NEAREST_GIT_BRANCH'"

if [[ "$SUFFIX_FROM_GIT_TAG" == "devnet" && "$NEAREST_GIT_BRANCH" == "dev" ]]; then
    echo "PASS: good match for -devnet releases"
    exit 0
fi

if [[ "$SUFFIX_FROM_GIT_TAG" == "testnet" && "$NEAREST_GIT_BRANCH" == "testnet" ]]; then
    echo "PASS: good match for -testnet releases"
    exit 0
fi

if [[ "$SUFFIX_FROM_GIT_TAG" == "mainnet" && "$NEAREST_GIT_BRANCH" == "main" ]]; then
    echo "PASS: good match for -mainnet releases"
    exit 0
fi

echo "FAIL: Looks like tag name doesn't match the branch it came from"
exit 1
