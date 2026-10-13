#!/usr/bin/env bash

# Compares the pallet storage versions of the local chain with those of a live network.
# Expects /var/tmp/pallet_storage.local and /var/tmp/pallet_storage.<network>, written by
# cli/dist/scripts/queryPalletVersions.js.
#
# Usage: compare-pallet-storage-versions.sh <network> <base-ref>
#
# On a PR into usc-testnet or main a difference fails the job: that is a promotion, and a
# storage version that moved without a migration would ship. On dev and asc-dev a
# difference is the normal state while migrations are pending release, so it is reported
# as a warning and in the job summary instead of turning every PR red.

set -euo pipefail

network="$1"
base_ref="$2"

diff_output=$(diff -u "/var/tmp/pallet_storage.${network}" /var/tmp/pallet_storage.local || true)

if [ -z "$diff_output" ]; then
  echo "Pallet storage versions match ${network}"
  exit 0
fi

echo "$diff_output" | colordiff || echo "$diff_output"

{
  echo "### Pallet storage versions differ from ${network}"
  echo
  echo '```diff'
  echo "$diff_output"
  echo '```'
} >> "${GITHUB_STEP_SUMMARY:-/dev/null}"

case "$base_ref" in
  usc-testnet | main)
    echo "::error title=Pallet storage versions::local chain differs from ${network}, see the job summary"
    exit 1
    ;;
  *)
    echo "::warning title=Pallet storage versions::local chain differs from ${network} (informational on ${base_ref:-this ref}), see the job summary"
    ;;
esac
