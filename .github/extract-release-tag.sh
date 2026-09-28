#!/bin/bash

set -euo pipefail

### find the latest release for one network suffix (devnet, asc-devnet, testnet, mainnet)
OWNER_REPO_SLUG="${GITHUB_REPOSITORY:-gluwa/creditcoin3}"
GREP_FOR="$1"
# Match the whole suffix: "devnet" must not pick up asc-devnet (or the old devnet-drynet)
# releases, which are separate networks with their own runtimes. No match prints nothing
# rather than failing, so a network's first release can fall back to another base.
RELEASE_TAG=$(curl --silent --header "Authorization: Bearer $GITHUB_TOKEN" "https://api.github.com/repos/$OWNER_REPO_SLUG/releases" | jq -r ".[].tag_name" | { grep -E "^v?[0-9]+\.[0-9]+\.[0-9]+-${GREP_FOR}\$" || true; } | head -n1)

echo "$RELEASE_TAG"
