#!/bin/bash

# Usage: network-for-ref.sh [--strict] <branch>
#
# Print the network whose releases a branch feeds:
#
#     main         ->  mainnet
#     usc-testnet  ->  testnet
#     asc-dev      ->  asc-devnet  (writeability-off-usc-dev until the rename)
#     dev          ->  devnet      (usc-dev until the rename)
#
# Any other branch, such as the base of a stacked pull request, is on its way into dev
# and maps to devnet. With --strict an unlisted branch fails instead, for callers that
# must not guess which chain they are testing against.

set -euo pipefail

STRICT=false
if [ "${1:-}" = "--strict" ]; then
    STRICT=true
    shift
fi

REF="${1:?usage: $0 [--strict] <branch>}"

case "$REF" in
    main) echo "mainnet" ;;
    usc-testnet) echo "testnet" ;;
    asc-dev | writeability-off-usc-dev) echo "asc-devnet" ;;
    dev | usc-dev) echo "devnet" ;;
    *)
        if [ "$STRICT" = true ]; then
            echo "FAIL: branch '$REF' does not feed a known network" >&2
            exit 1
        fi
        echo "devnet"
        ;;
esac
