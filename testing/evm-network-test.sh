#!/usr/bin/env bash

# WARNING:
# execute this script from inside its parent directory


set -e

TARGET_URL=$1
PRIVATE_KEY=$2

if [[ -z "$TARGET_URL" || -z "$PRIVATE_KEY" ]]; then
    echo "+++++ ERROR: Missing parameters"
    echo "+++++ usage: $0 <target-url> <private-key>"
    exit 1
fi

echo "+++++ Install smart contract test tool"
npm install

echo "+++++ Execute basicTest"
./node_modules/.bin/basicTest --rpc "$TARGET_URL" --private_key "$PRIVATE_KEY"

echo "++++ Execute stateOverrideTest"
./node_modules/.bin/stateOverrideTest --rpc "$TARGET_URL" --private_key "$PRIVATE_KEY"

echo "++++ Execute gasTest"
./node_modules/.bin/gasTest --rpc "$TARGET_URL" --private_key "$PRIVATE_KEY"

# Not ready, https://gluwa.slack.com/archives/C03MQ532BGA/p1698247767108099?thread_ts=1698217688.874159&cid=C03MQ532BGA
# echo "++++ Execute advancedTest"
# ./node_modules/.bin/advancedTest --rpc "$TARGET_URL" --private_key "$PRIVATE_KEY"
