#!/bin/bash

set -x

CONTAINER_NAME="$1"

# sleep 0-10hrs before actually doing anything
sleep $((RANDOM % 36000))

docker pause "$CONTAINER_NAME"

# sleep 0-5hrs before staring back up
sleep $((RANDOM % 18000))

docker unpause "$CONTAINER_NAME"
