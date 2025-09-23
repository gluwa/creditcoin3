#!/bin/bash

set -x

CONTAINER_NAME="$1"

# sleep 0-4hrs
sleep $((RANDOM % 14400))

# chill this attestor but don't kill the binary
creditcoin attestor chill \
    --url "$CREDITCOIN_RPC_URL" \
    --chain "$SEPOLIA_CHAIN_KEY" \
    --attestor "$ATTESTOR_ADDRESS"

# sleep 0-4hrs
sleep $((RANDOM % 14400))

# start this attestor again
# NOTE: will call `attest` extrinsic implicitly
docker restart "$CONTAINER_NAME"

# sleep 0-4hrs
sleep $((RANDOM % 14400))

# simulate a network disconnect w/o chill
docker stop "$CONTAINER_NAME"

# sleep 0-4hrs
sleep $((RANDOM % 14400))

# simulate network reconnect/start w/o chill
docker start "$CONTAINER_NAME"
