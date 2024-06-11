#!/bin/bash

set -euo pipefail

# this will be overriden when PRs are opened against different branches
TARGET_CHAIN=${TARGET_CHAIN:-devnet}
echo "INFO: will inspect 'precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json' file"

SRC_FROM_DISK=$(cat precompiles/metadata/sol/*.sol)
# NOTE: jq will interpret escape sequences so this value should equal to the raw code on disk
SRC_FROM_JSON=$(jq -r .[].source "precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json")

if [ "$SRC_FROM_DISK" == "$SRC_FROM_JSON" ]; then
    echo "INFO: Sources on disk match sources in JSON file"
else
    echo "FAIL: Sources on disk differ from sources in JSON file"
    echo "========================"

    echo "FROM_DISK=$SRC_FROM_DISK"
    echo "========================"
    echo "FROM_JSON=$SRC_FROM_JSON"
    echo "========================"

    exit 1
fi

ADDRESS_FROM_DISK=$(grep "address constant" precompiles/metadata/sol/*.sol | cut -f2 -d'=' | tr -d ' ;')
ADDRESS_FROM_JSON=$(jq -r .[].address "precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json")

if [ "$ADDRESS_FROM_DISK" == "$ADDRESS_FROM_JSON" ]; then
    echo "INFO: Address on disk matches address in JSON file"
else
    echo "FAIL: Address on disk differs from address in JSON file"

    echo "FROM_DISK=$ADDRESS_FROM_DISK"
    echo "FROM_JSON=$ADDRESS_FROM_JSON"
    echo "========================"

    exit 2
fi


# NOTE: requires that abi-creator.sh was executed beforehand
# NOTE2: both representations are multi-line
ABI_FROM_DISK=$(jq -r "..|.abi?|select(.)" precompiles/metadata/abi/*.json)
ABI_FROM_JSON=$(jq -r .[].abi "precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json" | jq -r)

if [ "$ABI_FROM_DISK" == "$ABI_FROM_JSON" ]; then
    echo "INFO: ABI on disk matches ABI in JSON file"
else
    echo "FAIL: ABI on disk differs from ABI in JSON file"

    echo "FROM_DISK=$ABI_FROM_DISK"
    echo "FROM_JSON=$ABI_FROM_JSON"
    echo "========================"

    exit 3
fi

ABI_FROM_TEST=$(cat cli/src/test/blockchain-tests/artifacts/SubstrateTransfer.json)
if [ "$ABI_FROM_DISK" == "$ABI_FROM_TEST" ]; then
    echo "INFO: ABI on disk matches ABI in tests"
else
    echo "FAIL: ABI on disk differs from ABI in tests"
    echo "TODO: Update the tests to make sure we're testing what we build"

    echo "FROM_DISK=$ABI_FROM_DISK"
    echo "FROM_TEST=$ABI_FROM_TEST"
    echo "========================"

    exit 4
fi
