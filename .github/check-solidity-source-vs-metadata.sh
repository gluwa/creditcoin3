#!/bin/bash

set -euo pipefail

# this will be overriden when PRs are opened against different branches
TARGET_CHAIN=${TARGET_CHAIN:-devnet}
echo "INFO: will inspect 'precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json' file"

# Concatenate all source files in alphabetical order (matching cat sol/*.sol behavior)
# cat naturally preserves newlines between files, so just use it directly
SRC_FROM_DISK=$(cat precompiles/metadata/sol/*.sol)
# Extract all sources from JSON and concatenate them
# NOTE: jq will interpret escape sequences so this value should equal to the raw code on disk
# Each source ends with a newline, so concatenating them naturally preserves separation
SRC_FROM_JSON=$(jq -r '.[].source' "precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json")

if [ "$SRC_FROM_DISK" == "$SRC_FROM_JSON" ]; then
    echo "INFO: Sources on disk match sources in JSON file"
else
    echo "FAIL: Sources on disk differ from sources in JSON file"
    echo "========================"

    echo "FROM_DISK=$SRC_FROM_DISK"
    echo "========================"
    echo "FROM_JSON=$SRC_FROM_JSON"
    echo "========================"

    diff -u <(echo "$SRC_FROM_DISK") <(echo "$SRC_FROM_JSON") | colordiff

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
# NOTE2: ABI files are now compact JSON arrays (single line)
# NOTE3: filter out empty ABIs (e.g., from libraries with no external functions)
# NOTE4: ABI in metadata JSON is stored as a JSON string, so decode it before comparing
# Flatten all ABIs from disk into a single array for comparison
ABI_FROM_DISK=$(jq -s '[.[] | select(length > 0)] | flatten' precompiles/metadata/abi/*.json | jq -r)
# Extract each ABI string, decode it, and collect into a single array
# Use jq to decode each ABI string and flatten into a single array
ABI_FROM_JSON=$(jq -r '[.[].abi | fromjson] | flatten' "precompiles/metadata/precompiles-creditcoin3-$TARGET_CHAIN.json" | jq -r)

if [ "$ABI_FROM_DISK" == "$ABI_FROM_JSON" ]; then
    echo "INFO: ABI on disk matches ABI in JSON file"
else
    echo "FAIL: ABI on disk differs from ABI in JSON file"

    echo "FROM_DISK=$ABI_FROM_DISK"
    echo "FROM_JSON=$ABI_FROM_JSON"
    echo "========================"

    diff -u <(echo "$ABI_FROM_DISK") <(echo "$ABI_FROM_JSON") | colordiff

    exit 3
fi
