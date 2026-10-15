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

expected="/var/tmp/pallet_storage.${network}"
actual=/var/tmp/pallet_storage.local

# A comparison that could not run is not a match: fail on promotions, warn elsewhere.
cannot_compare() {
  echo "$1" >&2
  case "$base_ref" in
    usc-testnet | main)
      echo "::error title=Pallet storage versions::could not compare with ${network}: $1"
      exit 1
      ;;
    *)
      echo "::warning title=Pallet storage versions::could not compare with ${network}: $1"
      exit 0
      ;;
  esac
}

for file in "$expected" "$actual"; do
  [ -s "$file" ] || cannot_compare "${file} is missing or empty"
done

# diff exits 0 when the files match, 1 when they differ and 2 on trouble.
diff_status=0
diff_output=$(diff -u "$expected" "$actual") || diff_status=$?
case "$diff_status" in
  0)
    echo "Pallet storage versions match ${network}"
    exit 0
    ;;
  1) ;;
  *) cannot_compare "diff failed with status ${diff_status}" ;;
esac

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
