#!/bin/bash

set -euo pipefail

SRC_FROM_DISK=$(cat precompiles/metadata/sol/*.sol)
# NOTE: jq will interpret escape sequences so this value should equal to the raw code on disk
SRC_FROM_JSON=$(jq -r .[0].source precompiles/metadata/precompiles-creditcoin3-devnet.json)

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


