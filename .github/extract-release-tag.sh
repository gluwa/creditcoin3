#!/bin/bash

set -euo pipefail

### find the latest release for testnet or mainnet
OWNER_REPO_SLUG="${GITHUB_REPOSITORY:-gluwa/creditcoin3}"
GREP_FOR="$1"
RELEASE_TAG=$(curl --silent --header "Authorization: Bearer $GITHUB_TOKEN" "https://api.github.com/repos/$OWNER_REPO_SLUG/releases" | jq -r ".[].tag_name" | grep "$GREP_FOR" | head -n1)

echo "$RELEASE_TAG"
